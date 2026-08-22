//! ログオン時に管理者権限で自動起動するためのタスク登録。
//!
//! Windows サービスとして登録する方法は使えない。サービスはセッション 0 で動作し、
//! ユーザーのデスクトップから隔離されるため、WinEvent フックもカーソル操作も
//! トレイアイコンも機能しないからである。
//!
//! 代わりにタスクスケジューラへ「ログオン時・最上位の特権で実行」のタスクを
//! 登録する。ユーザーセッション内で動くのでこれらの制約を受けない。
//!
//! 登録には管理者権限が必要なため、非昇格で実行中は自分自身を昇格して
//! 起動し直し、コマンドライン引数で処理させる。

use std::ffi::c_void;
use std::io::{Read, Write};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::process::CommandExt;
use std::process::Command;
use std::ptr::null_mut;
use std::sync::Mutex;

use windows_sys::Win32::Foundation::CloseHandle;
use windows_sys::Win32::System::Threading::GetCurrentProcess;
use windows_sys::Win32::UI::Shell::ShellExecuteW;

use crate::config::exe_dir;
use crate::util::wide;

/// 登録するタスク名
pub const TASK_NAME: &str = "DialogCursorMover";

/// コンソールウィンドウを出さずに起動する
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 他プロセスに読み取りだけを許す共有モード。
/// 書き込みも削除も拒否されるため、開いている間ファイルの中身は固定される。
const FILE_SHARE_READ: u32 = 0x0000_0001;

pub const ARG_INSTALL: &str = "--install-task";
pub const ARG_UNINSTALL: &str = "--uninstall-task";
/// ログオン時のタスクから起動されたことを示す引数。
///
/// これが無いと「タスクから起動された」ことを判別できず、既に別の
/// インスタンスが動いている場合にログオンのたびにモーダルダイアログが出る。
pub const ARG_AUTOSTART: &str = "--autostart";

const TOKEN_QUERY: u32 = 0x0008;
/// TokenElevation
const TOKEN_ELEVATION_CLASS: i32 = 20;

#[repr(C)]
struct TokenElevation {
    is_elevated: u32,
}

#[link(name = "advapi32")]
extern "system" {
    fn OpenProcessToken(process: *mut c_void, access: u32, token: *mut *mut c_void) -> i32;
    fn GetTokenInformation(
        token: *mut c_void,
        class: i32,
        info: *mut c_void,
        len: u32,
        ret_len: *mut u32,
    ) -> i32;
}

/// 現在のプロセスが管理者権限で動いているか。
pub unsafe fn is_elevated() -> bool {
    let mut token: *mut c_void = null_mut();
    if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
        return false;
    }
    let mut info = TokenElevation { is_elevated: 0 };
    let mut ret_len: u32 = 0;
    let ok = GetTokenInformation(
        token,
        TOKEN_ELEVATION_CLASS,
        &mut info as *mut _ as *mut c_void,
        std::mem::size_of::<TokenElevation>() as u32,
        &mut ret_len,
    );
    CloseHandle(token);
    ok != 0 && info.is_elevated != 0
}

/// 自分自身を管理者権限で起動し直し、指定の引数を渡す。
///
/// UAC のプロンプトが出る。承認されれば別プロセスで処理が実行される。
pub unsafe fn relaunch_elevated(arg: &str) -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let verb = wide("runas");
    let file = wide(&exe.to_string_lossy());
    let params = wide(arg);
    let result = ShellExecuteW(
        null_mut(),
        verb.as_ptr(),
        file.as_ptr(),
        params.as_ptr(),
        std::ptr::null(),
        0, // SW_HIDE
    );
    // ShellExecuteW は成功時に 32 より大きい値を返す
    result as isize > 32
}

fn schtasks(args: &[&str]) -> Result<(), String> {
    // 絶対パスで起動する。修飾されていない名前だと、CreateProcess が
    // システムディレクトリより先にカレントディレクトリを探してしまう。
    let program = crate::util::system32_path("schtasks.exe");

    // /Query は「登録されていないこと」の確認にも使うため、失敗しても異常ではない。
    // 通常ログに残すのは登録と削除だけにして、照会は詳細ログに留める。
    let level = if matches!(args.first(), Some(&"/Query")) {
        crate::log::VERBOSE
    } else {
        crate::log::NORMAL
    };
    crate::log::write(level, &format!("schtasks: 実行 {}", args.join(" ")));

    let output = Command::new(&program)
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| {
            crate::log::info(&format!(
                "schtasks: 起動できません path=\"{program}\" ({e})"
            ));
            format!("schtasks.exe を実行できません: {e}")
        })?;

    if output.status.success() {
        crate::log::write(level, "schtasks: 成功");
        return Ok(());
    }
    // schtasks の出力は OEM コードページなので、そのままでは文字化けする。
    // 判断に使うのは終了コードだけにする。
    let code = output.status.code().unwrap_or(-1);
    crate::log::write(level, &format!("schtasks: 失敗 終了コード={code}"));
    Err(format!("schtasks.exe が失敗しました (終了コード {code})"))
}

/// 登録状態のキャッシュ。
///
/// `schtasks.exe` の起動には数百 ms かかるため、メニューを開くたびに
/// 照会していると表示が目に見えて遅くなる。
static REGISTERED_CACHE: Mutex<Option<bool>> = Mutex::new(None);

/// タスクが登録済みか。照会は管理者権限がなくても行える。
pub fn is_registered() -> bool {
    if let Ok(cache) = REGISTERED_CACHE.lock() {
        if let Some(v) = *cache {
            return v;
        }
    }
    let registered = schtasks(&["/Query", "/TN", TASK_NAME]).is_ok();
    if let Ok(mut cache) = REGISTERED_CACHE.lock() {
        *cache = Some(registered);
    }
    registered
}

/// 登録状態のキャッシュを破棄する。登録／削除の直後に呼ぶ。
pub fn invalidate_cache() {
    if let Ok(mut cache) = REGISTERED_CACHE.lock() {
        *cache = None;
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn task_xml() -> Result<String, String> {
    let exe =
        std::env::current_exe().map_err(|e| format!("実行ファイルのパスを取得できません: {e}"))?;
    let exe = xml_escape(&exe.to_string_lossy());
    let dir = xml_escape(&exe_dir().to_string_lossy());

    // ログオントリガーと実行主体を、現在のユーザーに限定する
    let user = match (
        std::env::var("USERDOMAIN").ok(),
        std::env::var("USERNAME").ok(),
    ) {
        (Some(domain), Some(name)) => format!("{domain}\\{name}"),
        (None, Some(name)) => name,
        _ => return Err("ユーザー名を取得できません".to_string()),
    };
    let user = xml_escape(&user);

    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>ダイアログのOKボタンへ自動でマウスカーソルを移動する常駐ユーティリティ</Description>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <UserId>{user}</UserId>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{user}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>HighestAvailable</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>false</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>
    <IdleSettings>
      <StopOnIdleEnd>false</StopOnIdleEnd>
      <RestartOnIdle>false</RestartOnIdle>
    </IdleSettings>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <Hidden>false</Hidden>
    <RunOnlyIfIdle>false</RunOnlyIfIdle>
    <WakeToRun>false</WakeToRun>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>7</Priority>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{exe}</Command>
      <Arguments>{ARG_AUTOSTART}</Arguments>
      <WorkingDirectory>{dir}</WorkingDirectory>
    </Exec>
  </Actions>
</Task>
"#
    ))
}

/// タスク定義の XML を一時ファイルへ書き出し、内容が固定されたことを
/// 確かめたうえで、読み取りハンドルとパスを返す。
///
/// このファイルは昇格した `schtasks.exe` に読ませる。書き出してから読まれるまでの
/// 間に第三者が中身を差し替えられると、「最上位の特権で実行するログオンタスク」に
/// 別の実行ファイルを登録できてしまい、利用者が本アプリに対して承認した昇格が
/// まったく別のものに使われる。`%TEMP%` は昇格していても利用者のフォルダーなので、
/// 同じ利用者として動く通常権限のプロセスが書き込める点に注意がいる。
///
/// 名前をランダムにするだけでは足りない。攻撃側は名前を推測するのではなく、
/// ディレクトリの変更通知でファイルが現れた瞬間を知ることができる。そのため
/// 次の順序で「作成後に書き換えられない」状態を作る。
///
/// 1. 推測できない名前で `create_new` を使って作る（先回りして置いておく攻撃を防ぐ）
/// 2. いったん閉じ、`FILE_SHARE_READ` だけを許して開き直す。
///    以降このファイルは、他のプロセスからは読めるだけで書き換えも削除もできない
/// 3. 読み戻して、書いた内容のままであることを確かめる
///    （1 と 2 の間に差し替えられていないことの確認）
///
/// 返したハンドルは `schtasks.exe` が読み終わるまで保持すること。
fn write_task_xml(xml: &str) -> Result<(std::fs::File, std::path::PathBuf), String> {
    // タスクスケジューラの XML は UTF-16 (BOM 付き) である必要がある
    let mut bytes: Vec<u8> = vec![0xFF, 0xFE];
    for unit in xml.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }

    let dir = std::env::temp_dir();
    let mut last_err = String::new();

    // 名前が衝突した場合に備えて何度か試す
    for _ in 0..8 {
        let path = dir.join(format!("dcm-task-{}.xml", unique_suffix()));

        // 1) 推測できない名前で新規に作る
        let mut file = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(f) => f,
            Err(e) => {
                last_err = e.to_string();
                continue;
            }
        };
        let written = file.write_all(&bytes);
        drop(file);
        if let Err(e) = written {
            let _ = std::fs::remove_file(&path);
            return Err(format!("一時ファイルを作成できません: {e}"));
        }

        // 2) 書き換えと削除を拒否する形で開き直す
        let mut locked = match std::fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(&path)
        {
            Ok(f) => f,
            Err(e) => {
                let _ = std::fs::remove_file(&path);
                return Err(format!("一時ファイルを開き直せません: {e}"));
            }
        };

        // 3) 閉じてから開き直すまでの間に差し替えられていないことを確かめる
        let mut current = Vec::new();
        let read = locked.read_to_end(&mut current);
        if let Err(e) = read {
            drop(locked);
            let _ = std::fs::remove_file(&path);
            return Err(format!("一時ファイルを読み戻せません: {e}"));
        }
        if current != bytes {
            drop(locked);
            let _ = std::fs::remove_file(&path);
            // ここに到達したら、書き出しと開き直しの間に第三者が中身を
            // 差し替えたということ。攻撃の痕跡なので必ず残す
            crate::log::info(&format!(
                "警告: タスク定義の一時ファイルが書き換えられていました path=\"{}\"",
                path.display()
            ));
            return Err("一時ファイルが書き換えられました".to_string());
        }

        return Ok((locked, path));
    }
    Err(format!("一時ファイルを作成できません: {last_err}"))
}

/// 実行ファイルが、管理者以外にも書き込める可能性が高い場所にあるか。
///
/// 正確に判定するには ACL を読む必要があるが、実際に問題になるのは
/// 「zip を展開したまま個人用フォルダーから使っている」場合なので、
/// ユーザープロファイルと一時フォルダーの配下かどうかで近似する。
/// 判定できないときは false を返す。誤検出で警告を出すほうが害が大きい。
pub fn exe_in_user_writable_location() -> bool {
    let Ok(exe) = std::env::current_exe().and_then(|p| p.canonicalize()) else {
        return false;
    };
    for var in ["USERPROFILE", "TEMP", "TMP", "PUBLIC"] {
        let Some(raw) = std::env::var_os(var) else {
            continue;
        };
        if raw.is_empty() {
            continue;
        }
        let Ok(base) = std::path::PathBuf::from(raw).canonicalize() else {
            continue;
        };
        if exe.starts_with(&base) {
            return true;
        }
    }
    false
}

/// 種を攪拌する。
///
/// 種をそのまま 16 進で並べてはいけない。時刻・アドレス・PID はビットの
/// 占める位置が重ならないため、並べるとそれぞれが元の値のまま名前に現れる。
/// 実際、以前の実装ではファイル名の上位 64 bit が昇格プロセスのスタック
/// アドレスそのもので、誰でも一覧できる一時フォルダーから ASLR の情報が
/// 読める状態だった。
///
/// 暗号論的な強度は要らない。1 bit の違いが全体に波及し、出力から入力を
/// 復元できなければよい。
fn mix(seed: u128) -> u128 {
    // 奇数の乗数（全単射になる）と、上位から下位へ戻す右シフトを組み合わせる
    const MULTIPLIER: u128 = 0x2545_F491_4F6C_DD1D_9E37_79B9_7F4A_7C15;
    // 種がすべて 0 でも意味のある名前になるようにする
    let mut v = seed ^ 0x9E37_79B9_7F4A_7C15_F39C_C060_5CED_C835;
    for _ in 0..3 {
        v ^= v >> 67;
        v = v.wrapping_mul(MULTIPLIER);
        v ^= v >> 41;
    }
    v
}

/// 一時ファイル名に使う、推測しにくい文字列。
///
/// 暗号論的な強度は要らない。狙ったファイル名を先に作っておく攻撃を
/// 成立させないだけの予測困難さがあればよい。
fn unique_suffix() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    // アドレス空間配置のランダム化を種として混ぜる
    let addr = &nanos as *const u128 as usize as u128;
    let pid = std::process::id() as u128;

    format!("{:032x}", mix(nanos ^ (addr << 64) ^ addr ^ (pid << 32)))
}

/// タスクを登録する（要管理者権限）。
pub fn install() -> Result<(), String> {
    let xml = task_xml()?;
    // ハンドルは schtasks が読み終わるまで手放さない。
    // 開いている間、このファイルは書き換えも削除もできない。
    let (handle, path) = write_task_xml(&xml)?;
    crate::log::info(&format!(
        "自動起動タスクを登録します name=\"{}\" xml=\"{}\"",
        TASK_NAME,
        path.display()
    ));

    let result = schtasks(&[
        "/Create",
        "/TN",
        TASK_NAME,
        "/XML",
        &path.to_string_lossy(),
        "/F",
    ]);

    // FILE_SHARE_DELETE を許していないので、閉じてから消す
    drop(handle);
    let _ = std::fs::remove_file(&path);
    invalidate_cache();
    match &result {
        Ok(()) => crate::log::info("自動起動タスクを登録しました"),
        Err(e) => crate::log::info(&format!("自動起動タスクを登録できませんでした: {e}")),
    }
    result
}

/// タスクを削除する（要管理者権限）。
pub fn uninstall() -> Result<(), String> {
    crate::log::info(&format!("自動起動タスクを削除します name=\"{TASK_NAME}\""));
    let result = schtasks(&["/Delete", "/TN", TASK_NAME, "/F"]);
    invalidate_cache();
    match &result {
        Ok(()) => crate::log::info("自動起動タスクを削除しました"),
        Err(e) => crate::log::info(&format!("自動起動タスクを削除できませんでした: {e}")),
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一時ファイル名が推測しにくく、呼ぶたびに変わること。
    ///
    /// 固定名だと、昇格した schtasks.exe に読ませる直前に
    /// 第三者がファイルを差し替えられてしまう。
    #[test]
    fn task_xml_file_names_are_unpredictable() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..100 {
            let s = unique_suffix();
            assert_eq!(s.len(), 32, "長さが違う: {s}");
            assert!(
                s.chars().all(|c| c.is_ascii_hexdigit()),
                "16 進数でない: {s}"
            );
            seen.insert(s);
        }
        assert!(seen.len() > 1, "呼ぶたびに同じ名前になっている");
    }

    /// 種の値がファイル名から読み取れないこと。
    ///
    /// 以前は種をそのまま並べていたため、ファイル名の上位 64 bit に
    /// 昇格プロセスのスタックアドレスがそのまま現れていた。
    #[test]
    fn the_seed_cannot_be_read_back_from_the_file_name() {
        for seed in [
            0x0000_00f3_ed4f_fa10_18cd_6257_84ff_3d3c_u128,
            0,
            1,
            u128::MAX,
        ] {
            let out = mix(seed);
            assert_ne!(out >> 64, seed >> 64, "上位 64 bit がそのまま残っている");
            assert_ne!(
                out & u128::from(u64::MAX),
                seed & u128::from(u64::MAX),
                "下位 64 bit がそのまま残っている"
            );

            // 1 bit 変えると出力の広い範囲が変わること。
            // 攪拌していなければ 1 bit しか変わらない
            for bit in 0..128 {
                let changed = (out ^ mix(seed ^ (1u128 << bit))).count_ones();
                assert!(
                    (30..=98).contains(&changed),
                    "seed={seed:032x} の bit {bit} を変えたときの変化が {changed} ビット"
                );
            }
        }
    }
}
