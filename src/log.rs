use std::cell::{Cell, RefCell};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::atomic::{AtomicU32, Ordering};

/// 0 = 記録しない / 1 = 通常 / 2 = 詳細
static LOG_LEVEL: AtomicU32 = AtomicU32::new(0);

pub const OFF: u32 = 0;
pub const NORMAL: u32 = 1;
pub const VERBOSE: u32 = 2;

/// ログファイルの上限。超えたら古い行を捨てて切り詰める
const MAX_BYTES: u64 = 1_000_000;
/// 切り詰め後に残す割合。半分残せば直近の記録は十分に追える
const KEEP_RATIO: u64 = 2;
/// ログ 1 行の上限。長すぎるタイトルでログが埋まらないようにする
const MAX_LINE_BYTES: usize = 4000;
/// 何行ごとにファイルの実在を確かめるか
const EXISTENCE_CHECK_INTERVAL: u32 = 50;

thread_local! {
    /// 開きっぱなしのログファイル。1 行ごとに open/close すると
    /// 詳細ログ時の I/O コストが跳ね上がるためキャッシュする
    static SINK: RefCell<Option<File>> = const { RefCell::new(None) };
    /// 現在のファイルサイズ。毎回 metadata() を呼ばずに済ませるため自前で数える
    static SIZE: Cell<u64> = const { Cell::new(0) };
    /// 書き込み回数。ファイルの実在確認を間引くために使う
    static WRITES: Cell<u32> = const { Cell::new(0) };
}

pub fn set_level(level: u32) {
    let new = level.min(VERBOSE);
    let old = LOG_LEVEL.swap(new, Ordering::Relaxed);
    // レベルを切り替えたらハンドルを手放す。
    //
    // 開いたままだと、オフにしている間にログを削除されたことに気づけない。
    // Windows では削除済みのファイルへの書き込みも成功してしまうため、
    // 書き込みエラーとしても検出できず、記録が残らない状態になる。
    if old != new {
        close();
    }
}

pub fn level() -> u32 {
    LOG_LEVEL.load(Ordering::Relaxed)
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct SystemTimeRaw {
    year: u16,
    month: u16,
    day_of_week: u16,
    day: u16,
    hour: u16,
    minute: u16,
    second: u16,
    milliseconds: u16,
}

#[link(name = "kernel32")]
extern "system" {
    fn GetLocalTime(lp_system_time: *mut SystemTimeRaw);
}

fn timestamp() -> String {
    let mut st = SystemTimeRaw::default();
    unsafe { GetLocalTime(&mut st) };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
        st.year, st.month, st.day, st.hour, st.minute, st.second, st.milliseconds
    )
}

/// ログ 1 行に収めるため、制御文字を可視な記号に置き換える。
fn sanitize(msg: &str) -> String {
    let mut out = String::with_capacity(msg.len());
    for c in msg.chars() {
        match c {
            '\r' => {}
            '\n' => out.push('\u{21B5}'), // ↵
            c if c.is_control() => out.push('\u{FFFD}'),
            c => out.push(c),
        }
        if out.len() > MAX_LINE_BYTES {
            out.push_str(" ...");
            break;
        }
    }
    out
}

/// 上限を超えたログを、古い行を捨てて切り詰める。
///
/// 行単位で毎回削るとファイル全体の書き直しが要るため、上限に達した時点で
/// 一度だけ後半を残す。おおよそ半分が消え、直近の記録は残る。
///
/// 分割位置はバイト数で決めるが、そのまま切ると 2 つの問題が起きる。
///
/// * 行の途中で切れ、先頭に意味をなさない断片が残る
/// * UTF-8 の文字の途中で切れる。ログには日本語のウィンドウタイトルが
///   入るため、1 文字 3 バイトの境界がずれて文字化けする
///
/// そこで目安の位置から次の改行を探し、その直後を先頭にする。
/// 改行は ASCII なのでマルチバイト文字の途中に現れることはなく、
/// 行の途中で切れることもない。
fn truncate_old_lines(path: &std::path::Path) -> std::io::Result<u64> {
    let data = std::fs::read(path)?;
    let keep_from = (data.len() as u64 / KEEP_RATIO) as usize;

    // 目安の位置以降で最初の改行を探し、その次のバイトから残す
    let start = match data[keep_from..].iter().position(|&b| b == b'\n') {
        Some(offset) => keep_from + offset + 1,
        // 後半に改行が無い（＝極端に長い 1 行）場合は諦めて全部捨てる
        None => data.len(),
    };

    // 切り詰めたことを本文に残す。何も書かないと、読む側からは
    // 「なぜか途中から始まっているログ」にしか見えない
    let mut out = Vec::with_capacity(data.len() - start + 64);
    out.extend_from_slice("[ここより前の古いログは上限超過のため削除されました]\r\n".as_bytes());
    out.extend_from_slice(&data[start..]);

    std::fs::write(path, &out)?;
    Ok(out.len() as u64)
}

fn open_sink() -> Option<File> {
    let path = crate::config::log_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .ok()?;
    SIZE.with(|c| c.set(file.metadata().map(|m| m.len()).unwrap_or(0)));
    Some(file)
}

/// 指定レベル以上が有効なときだけログを追記する。
pub fn write(required: u32, msg: &str) {
    if required == OFF || level() < required {
        return;
    }
    // ログにはウィンドウタイトルなど他プロセスが決めた文字列が入る。
    // 改行を含むタイトルを付けたアプリがあると、ログ行を偽造できてしまうため、
    // 制御文字を落としてから 1 行として書き出す。
    let line = format!("[{}] {}\r\n", timestamp(), sanitize(msg));

    SINK.with(|cell| {
        let mut slot = cell.borrow_mut();

        // サイズ超過なら、いったんハンドルを閉じてから古い行を捨てる。
        // 開いたまま書き換えると、ハンドルが持つ書き込み位置と実体がずれる。
        if slot.is_some() && SIZE.with(|c| c.get()) > MAX_BYTES {
            *slot = None;
            let path = crate::config::log_path();
            match truncate_old_lines(&path) {
                Ok(remaining) => SIZE.with(|c| c.set(remaining)),
                // 切り詰められない場合は従来どおり作り直す
                Err(_) => {
                    let _ = std::fs::remove_file(&path);
                    SIZE.with(|c| c.set(0));
                }
            }
        }

        // 外部から削除された場合に備え、ときどき実在を確かめる。
        // 毎回 stat すると詳細ログでは負荷になるため、一定行ごとに絞る。
        if slot.is_some() {
            let n = WRITES.with(|c| {
                let v = c.get() + 1;
                c.set(v);
                v
            });
            if n.is_multiple_of(EXISTENCE_CHECK_INTERVAL) && !crate::config::log_path().exists() {
                *slot = None;
                SIZE.with(|c| c.set(0));
            }
        }

        if slot.is_none() {
            *slot = open_sink();
        }

        if let Some(file) = slot.as_mut() {
            // バッファリングしない。異常終了しても直前までのログが残るようにするため
            if file.write_all(line.as_bytes()).is_ok() {
                SIZE.with(|c| c.set(c.get() + line.len() as u64));
            } else {
                // 書き込めなくなったら次回開き直す
                *slot = None;
            }
        }
    });
}

/// 終了時にハンドルを解放する。
pub fn close() {
    SINK.with(|cell| *cell.borrow_mut() = None);
    SIZE.with(|c| c.set(0));
    WRITES.with(|c| c.set(0));
}

pub fn info(msg: &str) {
    write(NORMAL, msg);
}

pub fn debug(msg: &str) {
    write(VERBOSE, msg);
}
