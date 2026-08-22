#![windows_subsystem = "windows"]

mod autostart;
mod config;
mod cursor;
mod dialog;
mod lang;
mod log;
mod ripple;
mod sound;
mod theme;
mod tray;
mod uia;
mod util;

use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    GetLastError, ERROR_ALREADY_EXISTS, HWND, LPARAM, LRESULT, WPARAM,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::{CreateMutexW, GetCurrentProcessId};
use windows_sys::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetAncestor, GetCursorPos,
    GetForegroundWindow, GetMessageW, GetWindowThreadProcessId, IsWindow, IsWindowVisible,
    KillTimer, MessageBoxW, PostMessageW, PostQuitMessage, RegisterClassW, RegisterWindowMessageW,
    SetCursorPos, SetTimer, TranslateMessage, IDYES, MB_ICONWARNING, MB_OK, MB_YESNO, MSG, WM_APP,
    WM_CONTEXTMENU, WM_DESTROY, WM_ENDSESSION, WM_LBUTTONUP, WM_RBUTTONUP, WM_TIMER, WNDCLASSW,
};

use config::Config;
use util::wide;

// ---- DPI 対応 -----------------------------------------------------------

/// DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2
const DPI_PER_MONITOR_AWARE_V2: isize = -4;

/// MSGFLT_ALLOW
const MSGFLT_ALLOW: u32 = 1;

#[link(name = "user32")]
extern "system" {
    fn SetProcessDpiAwarenessContext(value: isize) -> i32;
    fn ChangeWindowMessageFilterEx(
        hwnd: *mut std::ffi::c_void,
        message: u32,
        action: u32,
        change_filter_struct: *mut std::ffi::c_void,
    ) -> i32;
}

/// プロセスをモニタ単位の DPI 対応にする。
///
/// これを呼ばないと、`GetWindowRect` や `SetCursorPos` は仮想化された論理座標を
/// 扱う一方、UI Automation の `CurrentBoundingRectangle` は物理座標を返すため、
/// 拡大率 100% 以外のディスプレイで移動先が実際のボタンから外れてしまう。
///
/// ウィンドウを作る前に呼ぶ必要がある。
///
/// ここは設定を読み込む前なのでログのレベルがまだ決まっていない。
/// 失敗を記録するのは呼び出し側の役目にして、成否だけを返す。
unsafe fn enable_dpi_awareness() -> bool {
    SetProcessDpiAwarenessContext(DPI_PER_MONITOR_AWARE_V2) != 0
}

// ---- 定数 ---------------------------------------------------------------

const WM_APP_TRAY: u32 = WM_APP + 1;
const WM_APP_DIALOG: u32 = WM_APP + 2;
const WM_APP_TEST: u32 = WM_APP + 3;
const WM_APP_FOCUS: u32 = WM_APP + 4;
const TIMER_ID: usize = 1;
/// 走行アニメーション用タイマー
const TIMER_ANIM: usize = 2;
/// トレイアイコン再登録用タイマー。
/// ログオン直後はタスクバーがまだ無く、登録に失敗することがある。
const TIMER_TRAY: usize = 4;
/// 再登録の間隔と上限（3 秒 × 20 回 = 約 1 分）
const TRAY_RETRY_INTERVAL_MS: u32 = 3000;
const TRAY_RETRY_LIMIT: u32 = 20;

/// フォーカス経路用タイマー。
/// ウィンドウ経路と共用すると、ダイアログ表示に伴うフォーカス移動が
/// 予約済みの処理を上書きしてしまうため、別のタイマーに分ける。
const TIMER_FOCUS: usize = 3;
/// アニメーションの更新間隔（約 60fps）
const ANIM_INTERVAL_MS: u32 = 16;
/// 走行時の揺れの周波数（Hz）。カーソルの脚の回転とおおよそ合う値
const WOBBLE_HZ: f64 = 4.0;
/// ティックの間隔がこれを超えたら「処理が滞った」とみなす
const TICK_STALL_LIMIT: Duration = Duration::from_millis(50);
/// カーソルが何ティック連続で進まなかったら移動を諦めるか。
/// 恒久的に拒否されているならこの先も動かないので、数ティックで確実に打ち切れる。
const MAX_STALLED_TICKS: u8 = 3;

const EVENT_SYSTEM_DIALOGSTART: u32 = 0x0010;
/// ウィンドウが前面になったとき。EVENT_OBJECT_SHOW が飛ばない、あるいは
/// 見逃したダイアログを拾うための保険
const EVENT_SYSTEM_FOREGROUND: u32 = 0x0003;
const EVENT_OBJECT_SHOW: u32 = 0x8002;
/// ウィンドウのタイトルが変わったとき。
/// エクスプローラのファイル操作ダイアログは、進捗表示から「ファイルの置換または
/// スキップ」へ同じウィンドウのまま切り替わるため、表示イベントが飛ばない。
/// タイトル変化がその唯一の手がかりになる。
const EVENT_OBJECT_NAMECHANGE: u32 = 0x800C;
/// フォーカスの移動。ウィンドウ内部に描画されるダイアログ
/// (WinUI の ContentDialog など) は新しい HWND を作らないため、
/// 「既定ボタンにフォーカスが移った」ことだけが手がかりになる。
const EVENT_OBJECT_FOCUS: u32 = 0x8005;
const WINEVENT_OUTOFCONTEXT: u32 = 0x0000;
const OBJID_WINDOW: i32 = 0;
const CHILDID_SELF: i32 = 0;

/// 同じダイアログを二重処理しないための抑止時間
const DEDUP_WINDOW: Duration = Duration::from_millis(1500);
/// UIA 走査は重いので、同じウィンドウに対する連続実行を間引く
const UIA_THROTTLE: Duration = Duration::from_millis(600);
/// 同じ座標へ繰り返し移動しないための抑止時間
const MOVE_DEDUP_WINDOW: Duration = Duration::from_millis(2500);
/// フォーカス変化は連続して飛ぶので間引く
const FOCUS_THROTTLE: Duration = Duration::from_millis(200);

/// pending の種別
const KIND_WINDOW: u8 = 0;
const KIND_UIA: u8 = 1;

const GA_ROOT: u32 = 2;

// ---- グローバル状態 -----------------------------------------------------

/// 走行アニメーションの進行状態
#[derive(Clone, Copy)]
struct Anim {
    from: (i32, i32),
    to: (i32, i32),
    start: Instant,
    duration: Duration,
    wobble: f64,
    /// 直前に自分で設定した座標。ユーザー操作の検出に使う
    last_set: (i32, i32),
    /// その 1 つ前に設定した座標。カーソルの追従遅れを見込むために使う
    prev_set: (i32, i32),
    /// 直前にティックを処理した時刻。処理が滞った場合の判定に使う
    last_tick: Instant,
    /// 音をまだ鳴らしていない。
    /// カーソルが実際に動いたことを確かめてから鳴らすために使う。
    sound_pending: bool,
    /// カーソルが進まなかったティックの連続回数。
    ///
    /// `SetCursorPos` は一時的に失敗することがある。実機のログでは、前面が
    /// 自分自身のウィンドウ（＝UIPI とは無関係）でも 30ms ほど連続して
    /// 失敗する例があった。1 回で諦めず、続けて動かない場合だけ打ち切る。
    stall_count: u8,
}

struct State {
    /// メッセージ受信用の隠しウィンドウ（usize として保持し Send を満たす）
    hwnd: usize,
    cfg: Config,
    /// タイマー満了時に処理する対象ダイアログ
    pending: usize,
    /// pending の種別（KIND_WINDOW / KIND_UIA）
    pending_kind: u8,
    /// 直近にフォーカス変化を処理した時刻
    last_focus: Option<Instant>,
    /// 最近処理したウィンドウ（重複抑止用）
    recent: Vec<(usize, Instant)>,
    /// 次に来る自プロセスのダイアログを 1 回だけ許可する（テスト用）
    allow_own_once: bool,
    /// 自プロセスのダイアログを一時的に抑止する（.wav 選択ダイアログの表示中など）。
    ///
    /// 真偽値ではなく入れ子の深さを数える。ピッカーを表示したままトレイから
    /// もう一枚開くと、内側が閉じた時点で外側の抑止まで解けてしまうため。
    suppress_own: u32,
    /// 直近に UIA 走査を予約したウィンドウと時刻
    last_uia: Option<(usize, Instant)>,
    /// 直近にカーソルを移動したウィンドウ・座標・時刻。
    ///
    /// ウィンドウも鍵に含める。座標だけだと、画面中央に出る別のダイアログが
    /// たまたま同じ位置にボタンを持っていた場合に取りこぼす。
    last_move: Option<(usize, (i32, i32), Instant)>,
    /// 既に移動済みのウィンドウ。
    /// 同じダイアログが再表示イベントを出しても繰り返し反応しないようにする
    handled: Vec<usize>,
    /// フォーカス経路で最後に反応したウィンドウ。
    ///
    /// ウィンドウ内部に描画されるダイアログはウィンドウを持たないため、
    /// `handled`（ウィンドウが閉じるまで保持）では抑止が強すぎる。
    /// ホストウィンドウを記録してしまい、アプリを終了するまで
    /// 二度と反応しなくなる。代わりにフォーカスの状態で判定する。
    focus_handled: Option<usize>,
    /// 進行中の走行アニメーション
    anim: Option<Anim>,
    /// トレイアイコン再登録の試行回数
    tray_retries: u32,
}

thread_local! {
    /// トレイメニューの処理中か。
    ///
    /// `show_menu` も `.wav` のピッカーもメッセージをポンプするため、
    /// その最中にトレイをもう一度クリックすると `on_tray_click` へ再入する。
    /// 内側で設定を変えても、外側は再入前のスナップショットから全体を
    /// 保存し直すので、変更がメモリ上もディスク上も消える。
    /// 外側が終わるまで、入れ子の呼び出しは無視する。
    static IN_TRAY_CLICK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

static STATE: Mutex<Option<State>> = Mutex::new(None);
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);
/// タイトル変化の監視を行うか（フック側から参照するので Atomic で持つ）
static WATCH_TITLES: AtomicBool = AtomicBool::new(true);
/// フォーカス変化の監視を行うか
static WATCH_FOCUS: AtomicBool = AtomicBool::new(true);

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    let mut guard = STATE.lock().ok()?;
    guard.as_mut().map(f)
}

// ---- エントリポイント ---------------------------------------------------

fn main() {
    // ログオン時のタスクから起動されたか
    let mut started_by_task = false;

    // タスク登録は昇格した別プロセスで行うため、
    // 二重起動チェックより前に引数を処理する
    if let Some(arg) = std::env::args().nth(1) {
        let cfg = Config::load();
        // ログのレベルを先に反映する。これが無いとレベルは 0（オフ）のままで、
        // 昇格した子プロセスが行うタスク登録の記録が一切残らない。
        // 利用者から届くのはログだけなので、いちばん失敗しうる処理が
        // 無記録になるのは避けたい
        log::set_level(cfg.log_level);
        // メッセージ表示に使うので言語も読む
        lang::load(&cfg.language);
        match arg.as_str() {
            autostart::ARG_INSTALL => {
                unsafe { report_task_result(autostart::install(), true) };
                return;
            }
            autostart::ARG_UNINSTALL => {
                unsafe { report_task_result(autostart::uninstall(), false) };
                return;
            }
            // 印を控えるだけで、処理は通常どおり続ける
            autostart::ARG_AUTOSTART => started_by_task = true,
            _ => {}
        }
    }
    unsafe { run(started_by_task) }
}

/// タスク登録／削除の結果をダイアログで知らせる。
unsafe fn report_task_result(result: Result<(), String>, installing: bool) {
    match result {
        Ok(()) => {
            let key = if installing {
                "msg.task.installed"
            } else {
                "msg.task.removed"
            };
            info_box(&lang::tf(key, &[autostart::TASK_NAME]));
        }
        Err(e) => {
            let key = if installing {
                "msg.task.install_failed"
            } else {
                "msg.task.remove_failed"
            };
            info_box(&lang::tf(key, &[&e]));
        }
    }
}

unsafe fn run(started_by_task: bool) {
    // 座標系を物理ピクセルに揃える。ウィンドウ作成より前に行う必要がある。
    // 結果の記録は log::set_level の後まで持ち越す
    let dpi_ok = enable_dpi_awareness();

    // 設定・ログ・言語を最初に確定させる。
    //
    // これより後ろに置くと、それまでの処理は「翻訳もログも効かない状態」で走る。
    // 実際、以前はここから下の 3 つの中止経路がすべて英語で表示され、
    // ログにも何も残らなかった。
    let cfg = Config::load();
    log::set_level(cfg.log_level);
    lang::load(&cfg.language);

    if !dpi_ok {
        log::info("DPI 対応の設定に失敗しました（拡大表示環境で座標がずれる可能性があります）");
    }

    if already_running() {
        log::info(&format!(
            "起動を中止しました: 既に別のインスタンスが動作しています (タスクからの起動={started_by_task})"
        ));
        // ログオン時のタスクから起動された場合はダイアログを出さない。
        // ログオンのたびにモーダルが 1 枚出るのは邪魔なだけで、
        // 利用者がそこで何かを判断できるわけでもない
        if !started_by_task {
            info_box(&lang::t("msg.already_running"));
        }
        return;
    }

    let hinst = GetModuleHandleW(std::ptr::null());
    let class_name = wide("DialogCursorMoverWndClass");

    let mut wc: WNDCLASSW = std::mem::zeroed();
    wc.lpfnWndProc = Some(wnd_proc);
    wc.hInstance = hinst;
    wc.lpszClassName = class_name.as_ptr();
    if RegisterClassW(&wc) == 0 {
        log::info(&format!(
            "起動を中止しました: ウィンドウクラスを登録できません (GetLastError={})",
            GetLastError()
        ));
        info_box(&lang::t("msg.class_failed"));
        return;
    }

    let title = wide("DialogCursorMover");
    let hwnd = CreateWindowExW(
        0,
        class_name.as_ptr(),
        title.as_ptr(),
        0, // 非表示のまま使う（ShowWindow は呼ばない）
        0,
        0,
        0,
        0,
        null_mut(),
        null_mut(),
        hinst,
        null_mut(),
    );
    if hwnd.is_null() {
        log::info(&format!(
            "起動を中止しました: ウィンドウを作成できません (GetLastError={})",
            GetLastError()
        ));
        info_box(&lang::t("msg.window_failed"));
        return;
    }

    // 前回異常終了してカーソルが差し替わったままなら、ここで元に戻す
    cursor::restore_stale_override();

    // UI Automation を使うため、メッセージループを持つこのスレッドで COM を初期化する
    if cfg.uia_enabled {
        uia::init_com();
    }

    {
        let mut guard = STATE.lock().unwrap();
        *guard = Some(State {
            hwnd: hwnd as usize,
            cfg: cfg.clone(),
            pending: 0,
            pending_kind: KIND_WINDOW,
            last_focus: None,
            recent: Vec::new(),
            allow_own_once: false,
            suppress_own: 0,
            last_uia: None,
            last_move: None,
            handled: Vec::new(),
            focus_handled: None,
            anim: None,
            tray_retries: 0,
        });
    }

    log::info(&format!(
        "起動しました (pid={}, enabled={}, delay={}ms, sound={}, standard_only={}, require_fg={})",
        GetCurrentProcessId(),
        cfg.enabled,
        cfg.delay_ms,
        cfg.sound_enabled,
        cfg.standard_dialog_only,
        cfg.require_foreground
    ));
    log::info(&format!(
        "設定・ログの保存先: {}",
        config::config_dir().display()
    ));

    // エクスプローラ再起動時にアイコンを貼り直すためのメッセージ
    let tc = RegisterWindowMessageW(wide("TaskbarCreated").as_ptr());
    TASKBAR_CREATED.store(tc, Ordering::Relaxed);

    // 昇格していると、非昇格の Explorer から送られる TaskbarCreated が
    // UIPI で遮断される。このメッセージだけ明示的に受け取れるようにする
    if ChangeWindowMessageFilterEx(hwnd as *mut _, tc, MSGFLT_ALLOW, null_mut()) == 0 {
        log::info("TaskbarCreated の受信許可に失敗しました");
    }

    ensure_tray_icon(hwnd, &cfg);

    // ダイアログ出現を捕まえる 2 種類のフック
    // 自プロセスの除外はフックフラグではなくソフト側で判定する（テスト用に一時解除できるようにするため）
    let flags = WINEVENT_OUTOFCONTEXT;
    let hook_dialog = SetWinEventHook(
        EVENT_SYSTEM_DIALOGSTART,
        EVENT_SYSTEM_DIALOGSTART,
        null_mut(),
        Some(win_event_proc),
        0,
        0,
        flags,
    );
    let hook_show = SetWinEventHook(
        EVENT_OBJECT_SHOW,
        EVENT_OBJECT_SHOW,
        null_mut(),
        Some(win_event_proc),
        0,
        0,
        flags,
    );
    // EVENT_SYSTEM_FOREGROUND も拾う。ウィンドウを使い回すアプリなど、
    // 表示イベントが飛ばないダイアログへの保険
    let hook_fg = SetWinEventHook(
        EVENT_SYSTEM_FOREGROUND,
        EVENT_SYSTEM_FOREGROUND,
        null_mut(),
        Some(win_event_proc),
        0,
        0,
        flags,
    );
    // 同じウィンドウのまま内容が差し替わるダイアログを拾うため、タイトル変化も見る
    WATCH_TITLES.store(cfg.watch_title_changes, Ordering::Relaxed);
    let hook_name = SetWinEventHook(
        EVENT_OBJECT_NAMECHANGE,
        EVENT_OBJECT_NAMECHANGE,
        null_mut(),
        Some(win_event_proc),
        0,
        0,
        flags,
    );
    // ウィンドウ内部に描画されるダイアログ用にフォーカス移動も見る
    WATCH_FOCUS.store(cfg.watch_focus_changes, Ordering::Relaxed);
    let hook_focus = SetWinEventHook(
        EVENT_OBJECT_FOCUS,
        EVENT_OBJECT_FOCUS,
        null_mut(),
        Some(win_event_proc),
        0,
        0,
        flags,
    );

    // メッセージループ
    let mut msg: MSG = std::mem::zeroed();
    while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }

    // 後始末
    if !hook_dialog.is_null() {
        UnhookWinEvent(hook_dialog);
    }
    if !hook_show.is_null() {
        UnhookWinEvent(hook_show);
    }
    if !hook_fg.is_null() {
        UnhookWinEvent(hook_fg);
    }
    if !hook_name.is_null() {
        UnhookWinEvent(hook_name);
    }
    if !hook_focus.is_null() {
        UnhookWinEvent(hook_focus);
    }
    sound::stop();
    ripple::stop();
    log::close();
    cursor::restore();
    // 通常は WM_DESTROY で消えている。GetMessageW がエラーで抜けた場合の保険
    tray::remove_icon(hwnd);
    uia::uninit_com();
}

unsafe fn already_running() -> bool {
    let name = wide("Local\\DialogCursorMover_SingleInstance_Mutex");
    let h = CreateMutexW(std::ptr::null(), 1, name.as_ptr());
    if h.is_null() {
        return false;
    }
    GetLastError() == ERROR_ALREADY_EXISTS
}

unsafe fn info_box(text: &str) {
    let t = wide(text);
    let c = wide("DialogCursorMover");
    // MB_ICONINFORMATION を付けると Windows が「メッセージ(情報)」のシステム音を鳴らすため、
    // アイコンなしの MB_OK にしている
    MessageBoxW(null_mut(), t.as_ptr(), c.as_ptr(), MB_OK);
}

/// カーソル移動が拒否されたときの状況を記録する。
///
/// 原因はたいてい UIPI で、前面のウィンドウが自分より高い整合性レベルで
/// 動いている場合に `SetCursorPos` が無視される。ただしそれを断定する材料は
/// 呼び出し側には無いので、推測を書くのではなく判断材料を残す。
/// 利用者から届くのはログだけなので、ここに何が写っているかが後で効く。
unsafe fn log_move_rejected(requested: (i32, i32), actual: (i32, i32)) {
    let fg = GetForegroundWindow();
    log::info(&format!(
        "中断: カーソルが動きませんでした 指示=({}, {}) 実際=({}, {}) 前面=\"{}\" プロセス=\"{}\" 自分の昇格={}",
        requested.0,
        requested.1,
        actual.0,
        actual.1,
        dialog::window_text(fg),
        dialog::process_name(fg).unwrap_or_else(|| "(取得できません)".to_string()),
        autostart::is_elevated(),
    ));
    // 前面が自分自身なら UIPI は関係ない。ここで昇格を勧めると誤った案内になる
    if !is_own_process(fg) {
        log::info(
            "前面のウィンドウが管理者権限で動いている場合、本アプリも管理者として実行すると回避できます",
        );
    }
}

/// はい／いいえを尋ねる。「はい」が選ばれたときだけ true を返す。
unsafe fn confirm_box(text: &str) -> bool {
    let t = wide(text);
    let c = wide("DialogCursorMover");
    MessageBoxW(
        null_mut(),
        t.as_ptr(),
        c.as_ptr(),
        MB_YESNO | MB_ICONWARNING,
    ) == IDYES
}

// ---- WinEvent フック ----------------------------------------------------

unsafe extern "system" fn win_event_proc(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    id_object: i32,
    id_child: i32,
    _thread: u32,
    _time: u32,
) {
    if hwnd.is_null() {
        return;
    }

    // フォーカスイベントはウィンドウ以外のオブジェクト (XAML の要素など) に
    // 対して飛ぶため、OBJID の絞り込みをかけてはいけない
    if event == EVENT_OBJECT_FOCUS {
        if !WATCH_FOCUS.load(Ordering::Relaxed) {
            return;
        }
        let target = with_state(|s| s.hwnd).unwrap_or(0);
        if target != 0 {
            PostMessageW(target as HWND, WM_APP_FOCUS, 0, 0);
        }
        return;
    }

    if id_object != OBJID_WINDOW || id_child != CHILDID_SELF {
        return;
    }
    if event != EVENT_OBJECT_SHOW
        && event != EVENT_SYSTEM_DIALOGSTART
        && event != EVENT_SYSTEM_FOREGROUND
        && event != EVENT_OBJECT_NAMECHANGE
    {
        return;
    }
    if event == EVENT_OBJECT_NAMECHANGE && !WATCH_TITLES.load(Ordering::Relaxed) {
        return;
    }
    // 実処理はメッセージループ側へ委譲する
    let target = with_state(|s| s.hwnd).unwrap_or(0);
    if target != 0 {
        PostMessageW(target as HWND, WM_APP_DIALOG, hwnd as usize as WPARAM, 0);
    }
}

// ---- ウィンドウプロシージャ ---------------------------------------------

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_APP_DIALOG => {
            on_dialog_shown(hwnd, wp);
            0
        }
        WM_TIMER if wp == TIMER_ID => {
            KillTimer(hwnd, TIMER_ID);
            on_timer();
            0
        }
        WM_TIMER if wp == TIMER_FOCUS => {
            KillTimer(hwnd, TIMER_FOCUS);
            if let Some(cfg) = with_state(|s| s.cfg.clone()) {
                on_focus_timer(&cfg);
            }
            0
        }
        WM_TIMER if wp == TIMER_TRAY => {
            if let Some(cfg) = with_state(|s| s.cfg.clone()) {
                ensure_tray_icon(hwnd, &cfg);
            }
            0
        }
        WM_TIMER if wp == TIMER_ANIM => {
            on_anim_tick(hwnd);
            0
        }
        WM_ENDSESSION => {
            // ログオフ・シャットダウン時にカーソルを戻し損ねないようにする。
            // wParam が FALSE のときはセッション終了が取り消された場合なので、
            // 走行中のアニメーションを止めてしまわない
            if wp != 0 {
                finish_move(hwnd);
            }
            0
        }
        WM_APP_FOCUS => {
            on_focus_changed(hwnd);
            0
        }
        WM_APP_TEST => {
            // 自プロセスのダイアログを 1 回だけ許可してから表示する
            with_state(|s| s.allow_own_once = true);
            info_box(&lang::t("msg.test_dialog"));
            with_state(|s| s.allow_own_once = false);
            0
        }
        WM_APP_TRAY => {
            let ev = (lp as u32) & 0xFFFF;
            if ev == WM_RBUTTONUP || ev == WM_LBUTTONUP || ev == WM_CONTEXTMENU {
                on_tray_click(hwnd);
            }
            0
        }
        WM_DESTROY => {
            ripple::stop();
            finish_move(hwnd);
            // ウィンドウが生きているうちに消す。破棄した後だと
            // Shell_NotifyIcon が (hWnd, uID) で対象を見つけられずに失敗し、
            // 通知領域にアイコンの残骸が残る
            tray::remove_icon(hwnd);
            PostQuitMessage(0);
            0
        }
        _ => {
            let tc = TASKBAR_CREATED.load(Ordering::Relaxed);
            if tc != 0 && msg == tc {
                // Explorer が再起動した、あるいはログオン直後にタスクバーができた
                with_state(|s| s.tray_retries = 0);
                if let Some(cfg) = with_state(|s| s.cfg.clone()) {
                    ensure_tray_icon(hwnd, &cfg);
                }
                return 0;
            }
            DefWindowProcW(hwnd, msg, wp, lp)
        }
    }
}

// ---- ダイアログ検出 -----------------------------------------------------

unsafe fn on_dialog_shown(self_hwnd: HWND, target: usize) {
    // ここはイベントごとに走るホットパスなので、Config は clone せず
    // ヒープ確保のないスナップショットだけを取り出す
    let Some(cfg) = with_state(|s| s.cfg.flags()) else {
        return;
    };

    if !cfg.enabled {
        return;
    }

    let hwnd = target as HWND;

    // 従来型ダイアログ (#32770 など) はそのまま対象にする
    let classic = dialog::is_candidate_dialog(hwnd, cfg.standard_dialog_only);

    // それ以外は UI Automation 経由の候補として、トップレベルの祖先を対象にする
    let (subject, via_uia) = if classic {
        (hwnd, false)
    } else if cfg.uia_enabled {
        // 自分が出したリップル表示は走査しない
        if dialog::class_is(hwnd, ripple::CLASS_NAME) {
            return;
        }
        let root = GetAncestor(hwnd, GA_ROOT);
        if root.is_null() {
            log::debug(&format!("UIA除外: 祖先を取得できません hwnd={:#x}", target));
            return;
        }
        if root == self_hwnd {
            return;
        }
        if IsWindowVisible(root) == 0 {
            // 非表示ウィンドウは大量に流れてくるうえ手の打ちようがないので記録しない
            return;
        }

        // 通常のアプリウィンドウ（ブラウザやエクスプローラ等）を走査すると
        // 数百〜数千要素を舐めることになり非常に重い。ダイアログらしい形の
        // ウィンドウだけに絞る
        if cfg.uia_dialog_like_only && !dialog::is_dialog_like(root) {
            log_uia_reject(root, "ダイアログらしい形ではありません");
            return;
        }

        // フォアグラウンド判定。
        // WPF の ShowDialog() などは「表示 → 活性化」の順なので、EVENT_OBJECT_SHOW
        // の時点ではまだ親ウィンドウが前面のまま。自分自身が前面でなくても
        // 「前面ウィンドウが所有しているダイアログ」なら対象にする。
        let fg = GetForegroundWindow();
        let owner = dialog::owner_of(root);
        // 前面そのものでも、前面が所有するダイアログでもないなら対象外
        if root != fg && (owner.is_null() || owner != fg) {
            log_uia_reject(root, "前面でもその所有ダイアログでもありません");
            return;
        }
        // 同じウィンドウへの連続走査を間引く
        let throttled = with_state(|s| {
            let now = Instant::now();
            match s.last_uia {
                Some((h, t)) if h == root as usize && now.duration_since(t) < UIA_THROTTLE => true,
                _ => {
                    s.last_uia = Some((root as usize, now));
                    false
                }
            }
        })
        .unwrap_or(true);
        if throttled {
            log::debug(&format!(
                "UIA除外: 直近に走査済みのためスキップ root={:#x} title=\"{}\"",
                root as usize,
                dialog::window_text(root)
            ));
            return;
        }
        (root, true)
    } else {
        log_uninteresting(target, hwnd);
        return;
    };

    // 自プロセスのダイアログの扱い。
    // ここまで来た（＝本当にダイアログだった）ものだけがテスト用の 1 回許可を消費する。
    if is_own_process(subject) {
        let (allowed, suppressed) = with_state(|s| {
            let allowed = s.allow_own_once;
            if allowed {
                s.allow_own_once = false;
            }
            (allowed, s.suppress_own > 0)
        })
        .unwrap_or((false, true));

        if !allowed && (cfg.ignore_own_process || suppressed) {
            log::debug(&format!(
                "自プロセスのダイアログのためスキップ hwnd={:#x} title=\"{}\" (ignore_own={} suppressed={})",
                subject as usize,
                dialog::window_text(subject),
                cfg.ignore_own_process,
                suppressed
            ));
            return;
        }
    }

    // 同じダイアログに対して DIALOGSTART と OBJECT_SHOW が二重で来るのを抑止する
    // (UIA 経路は別のスロットリングがあるので対象外)
    if !via_uia {
        let duplicate = with_state(|s| {
            let now = Instant::now();
            s.recent
                .retain(|(_, t)| now.duration_since(*t) < DEDUP_WINDOW);
            if s.recent.iter().any(|(h, _)| *h == subject as usize) {
                true
            } else {
                s.recent.push((subject as usize, now));
                if s.recent.len() > 64 {
                    s.recent.remove(0);
                }
                false
            }
        })
        .unwrap_or(true);
        if duplicate {
            return;
        }
    }

    log::info(&format!(
        "{}検出 hwnd={:#x} class=\"{}\" title=\"{}\" → {}ms 後に移動",
        if via_uia {
            "UIA候補"
        } else {
            "ダイアログ"
        },
        subject as usize,
        dialog::class_name(subject),
        dialog::window_text(subject),
        cfg.delay_ms
    ));

    with_state(|s| {
        s.pending = subject as usize;
        s.pending_kind = if via_uia { KIND_UIA } else { KIND_WINDOW };
    });
    // 0ms 指定でもレイアウト確定を待つため最低 1ms のタイマーを挟む
    KillTimer(self_hwnd, TIMER_ID);
    SetTimer(self_hwnd, TIMER_ID, cfg.delay_ms.max(1), None);
}

/// フォーカスが移ったときの処理を予約する。
unsafe fn on_focus_changed(self_hwnd: HWND) {
    let Some((enabled, uia_enabled, delay)) =
        with_state(|s| (s.cfg.enabled, s.cfg.uia_enabled, s.cfg.delay_ms))
    else {
        return;
    };
    if !enabled || !uia_enabled {
        return;
    }

    // フォーカス変化は連続して飛ぶので間引く
    let throttled = with_state(|s| {
        let now = Instant::now();
        match s.last_focus {
            Some(t) if now.duration_since(t) < FOCUS_THROTTLE => true,
            _ => {
                s.last_focus = Some(now);
                false
            }
        }
    })
    .unwrap_or(true);
    if throttled {
        return;
    }

    // ウィンドウ経路のタイマーには触れない
    KillTimer(self_hwnd, TIMER_FOCUS);
    SetTimer(self_hwnd, TIMER_FOCUS, delay.max(1), None);
}

/// フォーカス中の要素が OK ボタンなら、そこへ移動する。
unsafe fn on_focus_timer(cfg: &Config) {
    let fg = GetForegroundWindow();
    if fg.is_null() {
        return;
    }

    // 同じウィンドウをウィンドウ経路が処理予定なら、そちらに任せる。
    //
    // 両経路は別のタイマーで動くため、フォーカス経路が数十 ms 早く走ることがある。
    // ウィンドウ経路のほうが除外判定（ファイルダイアログ、処理中ダイアログなど）が
    // 充実しているうえ、判定が遅いぶんウィンドウの状態も安定している。
    if with_state(|s| s.pending == fg as usize).unwrap_or(false) {
        log::debug("フォーカス: ウィンドウ経路が処理予定のため見送ります");
        return;
    }

    // 自プロセスのダイアログの扱いはウィンドウ経路と揃える
    if is_own_process(fg) {
        let (allowed, suppressed) = with_state(|s| {
            let allowed = s.allow_own_once;
            if allowed {
                s.allow_own_once = false;
            }
            (allowed, s.suppress_own > 0)
        })
        .unwrap_or((false, true));
        if !allowed && (cfg.ignore_own_process || suppressed) {
            return;
        }
    }

    if !cfg.exclude_titles.is_empty() {
        let title = dialog::window_text(fg);
        if cfg
            .exclude_titles
            .iter()
            .any(|k| config::contains_ignore_ascii_case(&title, k))
        {
            return;
        }
    }
    if !cfg.exclude_processes.is_empty() {
        if let Some(exe) = dialog::process_name(fg) {
            if cfg
                .exclude_processes
                .iter()
                .any(|p| p.eq_ignore_ascii_case(&exe))
            {
                return;
            }
        }
    }
    if cfg.skip_file_dialogs && dialog::is_file_dialog(fg) {
        return;
    }
    // ウィンドウ経路と判定を揃える
    if cfg.skip_progress_dialogs && dialog::has_progress_bar(fg) {
        return;
    }

    let Some(focused) = uia::focused_button(&cfg.extra_button_labels) else {
        // フォーカスがボタン以外へ移った＝ダイアログが閉じたとみなし、
        // 次に同じウィンドウでダイアログが出たら再び反応できるようにする
        with_state(|s| s.focus_handled = None);
        return;
    };

    // ラベルが登録済みのものと一致しない場合は、
    // 「ダイアログのボタン行にいるか」で既定ボタンかどうかを判定する
    let via = if focused.label_matched {
        "FOCUS"
    } else {
        if !cfg.follow_dialog_default_button {
            log::debug(&format!(
                "フォーカス: ラベル未登録のボタンです \"{}\"",
                focused.name
            ));
            return;
        }
        // ブラウザでは配置による既定ボタン判定を行わない。
        // Web ページは任意の UI を作れるため、「ブロック / キャンセル」のような
        // 並びがダイアログのボタン行と区別できず、重い操作に反応してしまう。
        if !cfg.follow_default_in_browser && dialog::is_browser_window(fg) {
            log::debug(&format!(
                "フォーカス: ブラウザのため既定ボタン判定を行いません \"{}\"",
                focused.name
            ));
            return;
        }
        if dialog::is_cancel_label(&focused.name) {
            log::debug(&format!(
                "フォーカス: キャンセル相当のため見送ります \"{}\"",
                focused.name
            ));
            return;
        }
        if !uia::has_cancel_sibling(fg, &focused.rect, cfg.uia_max_elements as usize) {
            log::debug(&format!(
                "フォーカス: ダイアログのボタン行ではありません \"{}\"",
                focused.name
            ));
            return;
        }
        "FOCUS(既定)"
    };

    log::info(&format!(
        "フォーカス検出 [{}] ボタン=\"{}\" (前面: \"{}\")",
        via,
        focused.name,
        dialog::window_text(fg)
    ));

    let found = focused;

    if cfg.skip_if_cursor_inside {
        let mut pt = std::mem::zeroed();
        if GetCursorPos(&mut pt) != 0 && dialog::point_in_rect(&pt, &found.rect) {
            log::debug("スキップ: 既にカーソルがボタン上にあります");
            return;
        }
    }

    let center = dialog::center_of(&found.rect);

    if !dialog::point_is_on_a_monitor(&center) {
        log::info(&format!(
            "中止: 移動先がどのモニタにも乗っていません ({}, {})",
            center.x, center.y
        ));
        return;
    }

    // 判定だけ行い、記録は実際に移動すると決まってから
    let repeated = with_state(|s| {
        let now = Instant::now();
        matches!(
            s.last_move,
            Some((w, p, t))
                if w == fg as usize
                    && p == (center.x, center.y)
                    && now.duration_since(t) < MOVE_DEDUP_WINDOW
        )
    })
    .unwrap_or(false);
    if repeated {
        log::debug("スキップ: 直前と同じ位置です");
        return;
    }

    // ウィンドウの寿命ではなくフォーカスの状態で抑止する。
    // 同じウィンドウでフォーカスが当たり続けている間は 1 回だけ反応し、
    // フォーカスが外れれば（＝ダイアログが閉じれば）また反応できる。
    if cfg.move_once_per_dialog {
        let already = with_state(|s| s.focus_handled == Some(fg as usize)).unwrap_or(false);
        if already {
            log::debug("スキップ: このダイアログには移動済みです");
            return;
        }
        with_state(|s| s.focus_handled = Some(fg as usize));
    }

    with_state(|s| s.last_move = Some((fg as usize, (center.x, center.y), Instant::now())));

    if !cfg.sound_enabled {
        log::debug("サウンド: 設定が無効なため再生しません");
    }
    start_move(center.x, center.y, cfg, via, &found.name, cfg.sound_enabled);
}

/// トレイアイコンを登録する。失敗した場合はタイマーで再試行する。
///
/// ログオン直後は Explorer がまだタスクバーを作っていないことがあり、
/// その間は `Shell_NotifyIcon(NIM_ADD)` が失敗する。
unsafe fn ensure_tray_icon(hwnd: HWND, cfg: &Config) {
    if tray::add_icon(hwnd, WM_APP_TRAY, cfg) {
        KillTimer(hwnd, TIMER_TRAY);
        // 再試行の末に成功した場合は、その旨を残しておく
        if with_state(|s| std::mem::replace(&mut s.tray_retries, 0)).unwrap_or(0) > 0 {
            log::info("トレイアイコンを登録しました");
        }
        return;
    }

    let retries = with_state(|s| {
        s.tray_retries += 1;
        s.tray_retries
    })
    .unwrap_or(u32::MAX);

    if retries > TRAY_RETRY_LIMIT {
        KillTimer(hwnd, TIMER_TRAY);
        log::info("トレイアイコンを登録できませんでした（再試行を打ち切りました）");
        return;
    }

    log::info(&format!(
        "トレイアイコンの登録に失敗しました。{}ms 後に再試行します ({}/{})",
        TRAY_RETRY_INTERVAL_MS, retries, TRAY_RETRY_LIMIT
    ));
    SetTimer(hwnd, TIMER_TRAY, TRAY_RETRY_INTERVAL_MS, None);
}

/// ログオン時の自動起動タスクを登録／削除する。
///
/// タスクの作成には管理者権限が必要なので、非昇格で動いている場合は
/// 自分自身を昇格して起動し直し、そちらに処理させる。
unsafe fn toggle_autostart() {
    let registered = autostart::is_registered();
    let arg = if registered {
        autostart::ARG_UNINSTALL
    } else {
        autostart::ARG_INSTALL
    };

    // 登録する前に置き場所を確かめる。
    //
    // 管理者以外も書き込めるフォルダーから「管理者として自動起動」を登録すると、
    // そのフォルダーに後から置かれたものが管理者権限で走るようになる。
    // UAC が守ろうとしている境界そのものなので、ここで一度だけ確認する。
    if !registered && autostart::exe_in_user_writable_location() {
        let dir = config::exe_dir().display().to_string();
        if !confirm_box(&lang::tf("msg.task.unsafe_location", &[&dir])) {
            log::info("自動起動の登録を中止しました（置き場所の確認で「いいえ」）");
            return;
        }
    }

    if autostart::is_elevated() {
        let result = if registered {
            autostart::uninstall()
        } else {
            autostart::install()
        };
        report_task_result(result, !registered);
        return;
    }

    // 非昇格の場合は UAC を経由して自分自身に処理させる
    if !autostart::relaunch_elevated(arg) {
        log::info("自動起動: 昇格が拒否されたため中止しました");
        info_box(&lang::t("msg.task.denied"));
        return;
    }
    log::info(&format!(
        "自動起動: 昇格した別プロセスへ {arg} を委譲しました"
    ));
    // 別プロセスが状態を変えたので、キャッシュを捨てて次回照会させる
    autostart::invalidate_cache();
    info_box(&lang::t("msg.task.elevated"));
}

/// このウィンドウに対して既に移動済みかを判定し、未処理なら記録する。
///
/// タブの切り替えや再アクティブ化で同じダイアログが何度も表示イベントを
/// 出すことがあるため、「1 つのダイアログにつき 1 回」に制限する。
///
/// 記録はウィンドウ単位で、移動先の座標は含めない。座標も鍵にすると、
/// ダイアログを画面上でドラッグしただけで別物と判定され、
/// アクティブにするたびに反応してしまう。
///
/// 記録はウィンドウが閉じられた時点で掃除されるので、
/// 閉じて開き直した場合は再び反応する。
unsafe fn claim_move(owner: HWND, once_per_dialog: bool) -> bool {
    if !once_per_dialog {
        return true;
    }
    let key = owner as usize;
    with_state(|s| {
        // 既に閉じたウィンドウの記録を捨てる
        s.handled.retain(|h| IsWindow(*h as HWND) != 0);
        if s.handled.contains(&key) {
            return false;
        }
        s.handled.push(key);
        if s.handled.len() > 64 {
            s.handled.remove(0);
        }
        true
    })
    .unwrap_or(true)
}

/// UIA 候補として不採用にした理由を、トップレベルウィンドウの情報付きで記録する。
///
/// イベント元が子ウィンドウの場合でも root の情報を出すので、
/// 「どのダイアログが、なぜ捨てられたか」が必ず追える。
unsafe fn log_uia_reject(root: HWND, reason: &str) {
    if log::level() < log::VERBOSE {
        return;
    }
    let owner = dialog::owner_of(root);
    log::debug(&format!(
        "UIA除外: {} root={:#x} class=\"{}\" title=\"{}\" owner={:#x} rect={:?}",
        reason,
        root as usize,
        dialog::class_name(root),
        dialog::window_text(root),
        owner as usize,
        dialog::window_rect(root).map(|r| (r.left, r.top, r.right - r.left, r.bottom - r.top))
    ));
}

/// 詳細ログ用。トップレベルの見慣れないウィンドウだけ記録する。
unsafe fn log_uninteresting(target: usize, hwnd: HWND) {
    if log::level() < log::VERBOSE || !dialog::is_top_level(hwnd) || IsWindowVisible(hwnd) == 0 {
        return;
    }
    let cls = dialog::class_name(hwnd);
    if cls.is_empty() {
        return;
    }
    log::debug(&format!(
        "対象外 hwnd={:#x} class=\"{}\" title=\"{}\"",
        target,
        cls,
        dialog::window_text(hwnd)
    ));
}

unsafe fn is_own_process(hwnd: HWND) -> bool {
    let mut pid: u32 = 0;
    GetWindowThreadProcessId(hwnd, &mut pid);
    pid != 0 && pid == GetCurrentProcessId()
}

unsafe fn on_timer() {
    let Some((target, cfg, kind)) = with_state(|s| (s.pending, s.cfg.clone(), s.pending_kind))
    else {
        return;
    };

    handle_pending_dialog(target, &cfg, kind);

    // pending は処理を終えてから消す。
    //
    // 冒頭で消すと、フォーカス経路の「ウィンドウ経路が処理予定か」という
    // 判定が常に空振りする。EVENT_OBJECT_SHOW が先に来る通常の順序では
    // こちらのタイマーが先に満了するため、フォーカス経路が見る頃には
    // 既に 0 になっているからである。
    //
    // この処理中は UIA 走査などでメッセージがポンプされ、別のダイアログが
    // 検出されて pending が置き換わっていることがある。自分が扱った対象の
    // ままのときだけ消す。
    with_state(|s| {
        if s.pending == target {
            s.pending = 0;
            s.pending_kind = KIND_WINDOW;
        }
    });
}

/// 遅延後に、検出済みのダイアログを実際に処理する。
unsafe fn handle_pending_dialog(target: usize, cfg: &Config, kind: u8) {
    let was_uia = kind == KIND_UIA;
    if target == 0 || !cfg.enabled {
        return;
    }

    let dlg = target as HWND;
    if IsWindow(dlg) == 0 || IsWindowVisible(dlg) == 0 {
        log::info(&format!(
            "中止: ウィンドウが既に閉じています hwnd={:#x}",
            target
        ));
        return;
    }
    // 最小化してもウィンドウは WS_VISIBLE のままなので、上の判定は通る。
    // 矩形は (-32000, -32000) 付近になるため、ここで弾かないと
    // ボタンの中心として画面外の座標が出てくる
    if dialog::is_minimized(dlg) {
        log::info(&format!(
            "中止: ダイアログが最小化されています hwnd={:#x}",
            target
        ));
        return;
    }
    if cfg.require_foreground {
        let fg = GetForegroundWindow();
        let owner = dialog::owner_of(dlg);
        // ダイアログ自身が前面か、前面ウィンドウが所有しているダイアログなら許可する
        // 前面そのものでも、前面が所有するダイアログでもないなら対象外
        if fg != dlg && (owner.is_null() || owner != fg) {
            log::info(&format!(
                "中止: ダイアログが前面ではありません hwnd={:#x} (require_foreground=false で無効化できます)",
                target
            ));
            return;
        }
    }

    // ウィンドウが確定してから、もう一度「ダイアログらしいか」を確かめる。
    //
    // 検出時点ではアプリの復帰・最大化の途中でサイズが未確定なことがあり、
    // 通常のアプリウィンドウが一瞬だけ小さく見えて候補に入ることがある。
    // 遅延の後なら形が落ち着いているので、ここで振るい落とす。
    if was_uia && cfg.uia_dialog_like_only && !dialog::is_dialog_like(dlg) {
        log::debug(&format!(
            "スキップ: 通常のウィンドウでした hwnd={:#x} title=\"{}\"",
            target,
            dialog::window_text(dlg)
        ));
        return;
    }

    // 除外判定は遅延の「後」に行う。表示直後は子コントロールが揃っていないことがあるため。
    if !cfg.exclude_titles.is_empty() {
        let title = dialog::window_text(dlg);
        if let Some(hit) = cfg
            .exclude_titles
            .iter()
            .find(|k| config::contains_ignore_ascii_case(&title, k))
        {
            log::info(&format!(
                "スキップ: 除外タイトルに一致 hwnd={:#x} title=\"{}\" (\"{}\")",
                target, title, hit
            ));
            return;
        }
    }
    if !cfg.exclude_processes.is_empty() {
        if let Some(exe) = dialog::process_name(dlg) {
            if cfg
                .exclude_processes
                .iter()
                .any(|p| p.eq_ignore_ascii_case(&exe))
            {
                log::info(&format!(
                    "スキップ: 除外プロセスです hwnd={:#x} exe=\"{}\"",
                    target, exe
                ));
                return;
            }
        }
    }
    if cfg.skip_file_dialogs && dialog::is_file_dialog(dlg) {
        log::info(&format!(
            "スキップ: ファイル/フォルダー選択ダイアログです hwnd={:#x} title=\"{}\"",
            target,
            dialog::window_text(dlg)
        ));
        return;
    }
    // HWND ベースの進捗表示はここで弾ける。UIA ベースのものは走査後に判定する
    if cfg.skip_progress_dialogs && dialog::has_progress_bar(dlg) {
        log::info(&format!(
            "スキップ: 処理中ダイアログです（プログレスバー検出） hwnd={:#x} title=\"{}\"",
            target,
            dialog::window_text(dlg)
        ));
        return;
    }
    let mut scan = uia::ScanInfo::default();

    // 各段は「ボタンが見つかり、かつ矩形も取れた」ときだけ成立させる。
    // if / else if で繋ぐと、ボタンは見つかったのに矩形が取れなかった場合に
    // 後段のフォールバックへ落ちなくなる。
    let found = dialog::find_ok_button(dlg, &cfg.extra_button_labels)
        .and_then(|b| dialog::window_rect(b).map(|r| (r, dialog::window_text(b), "HWND")))
        .or_else(|| {
            if !cfg.follow_dialog_default_button {
                return None;
            }
            // ブラウザ系ウィンドウでは配置による既定ボタン判定を行わない
            if !cfg.follow_default_in_browser && dialog::is_browser_window(dlg) {
                return None;
            }
            // ラベル未登録でも、キャンセルと並んだ既定ボタンならダイアログとみなす
            dialog::find_default_button(dlg).and_then(|b| {
                dialog::window_rect(b).map(|r| (r, dialog::window_text(b), "HWND(既定)"))
            })
        })
        .or_else(|| {
            if !cfg.uia_enabled {
                return None;
            }
            uia::find_ok_button(
                dlg,
                &cfg.extra_button_labels,
                cfg.uia_max_elements as usize,
                &mut scan,
            )
            .map(|f| (f.rect, f.name, "UIA"))
        });

    // UIA 走査中にプログレスバーを見つけていたら、ボタンが取れていても見送る
    if cfg.skip_progress_dialogs && scan.progress {
        log::info(&format!(
            "スキップ: 処理中ダイアログです（UIA でプログレスバー検出） hwnd={:#x} title=\"{}\"",
            target,
            dialog::window_text(dlg)
        ));
        return;
    }

    let Some((rect, label, via)) = found else {
        // UIA 候補は「たまたま前面に出ただけの普通のウィンドウ」も多いので詳細ログ扱い
        let required = if was_uia { log::VERBOSE } else { log::NORMAL };
        if log::level() >= required {
            // 見つかったボタン名を出す。extra_button_labels に何を書けばよいか分かる
            let candidates = if scan.seen.is_empty() {
                "(UIA でも押せる要素は見つかりません)".to_string()
            } else {
                format!("UIA で見つかったボタン: {}", scan.seen.join(" / "))
            };
            // dump_children は重いのでログを出すときだけ組み立てる
            log::write(
                required,
                &format!(
                    "中止: OK ボタンが見つかりません hwnd={:#x} {} 子: {}",
                    target,
                    candidates,
                    dialog::dump_children(dlg)
                ),
            );
        }
        return;
    };

    if cfg.skip_if_cursor_inside {
        let mut pt = std::mem::zeroed();
        if GetCursorPos(&mut pt) != 0 && dialog::point_in_rect(&pt, &rect) {
            log::debug("スキップ: 既にカーソルがボタン上にあります");
            return;
        }
    }

    let center = dialog::center_of(&rect);

    // 送る直前の最後の関門。どのモニタにも乗っていない座標へ送っても
    // 画面の隅に張り付くだけで、利用者には何が起きたのか分からない
    if !dialog::point_is_on_a_monitor(&center) {
        log::info(&format!(
            "中止: 移動先がどのモニタにも乗っていません ({}, {}) hwnd={:#x}",
            center.x, center.y, target
        ));
        return;
    }

    // 同じ場所へ短時間に何度も飛ばないようにする（UIA 経路で効く）
    // 判定だけ行い、記録は実際に移動すると決まってから
    let repeated = with_state(|s| {
        let now = Instant::now();
        matches!(
            s.last_move,
            Some((w, p, t))
                if w == dlg as usize
                    && p == (center.x, center.y)
                    && now.duration_since(t) < MOVE_DEDUP_WINDOW
        )
    })
    .unwrap_or(false);
    if repeated {
        log::debug("スキップ: 直前と同じ位置です");
        return;
    }

    if !claim_move(dlg, cfg.move_once_per_dialog) {
        log::debug(&format!(
            "スキップ: このダイアログには移動済みです hwnd={:#x}",
            target
        ));
        return;
    }

    with_state(|s| s.last_move = Some((dlg as usize, (center.x, center.y), Instant::now())));

    if !cfg.sound_enabled {
        log::debug("サウンド: 設定が無効なため再生しません");
    }

    // 音は start_move の中で、カーソルが実際に動いてから鳴らす
    start_move(center.x, center.y, cfg, via, &label, cfg.sound_enabled);
}

/// カーソル移動を開始する。設定に応じて走行アニメーションか瞬間移動になる。
///
/// 音は移動を確かめてから鳴らす。UIPI で `SetCursorPos` が拒否される環境では
/// カーソルが動かないため、先に鳴らすと「音は出るのに動かない」ことになる。
unsafe fn start_move(x: i32, y: i32, cfg: &Config, via: &str, label: &str, play_sound: bool) {
    let self_hwnd = with_state(|s| s.hwnd).unwrap_or(0) as HWND;

    // 走行中に次の移動が始まることがある。古いアニメーションをここで畳まないと、
    // 瞬間移動の分岐に入ったときに生き残ったタイマーが元の目標へカーソルを
    // 引き戻してしまう。
    //
    // 差し替えたカーソルはここでは戻さない。続けて走る場合は作り直しになり、
    // 一瞬だけ既定のカーソルに戻って見えるため。代わりに、走らないと決まった
    // 経路（下の瞬間移動と各中断）で必ず finish_move を呼んで後始末する。
    if !self_hwnd.is_null() {
        KillTimer(self_hwnd, TIMER_ANIM);
    }
    with_state(|s| s.anim = None);

    let mut from = std::mem::zeroed();
    if GetCursorPos(&mut from) == 0 {
        log::info("中止: 現在のカーソル位置を取得できません");
        finish_move(self_hwnd);
        return;
    }

    let dx = (x - from.x) as f64;
    let dy = (y - from.y) as f64;
    let distance = (dx * dx + dy * dy).sqrt();

    // アニメーション無効、時間 0、距離がごく短い場合は従来どおり一気に動かす
    if !cfg.move_animation || cfg.move_duration_ms == 0 || distance < 8.0 {
        if SetCursorPos(x, y) == 0 {
            log::info("中止: SetCursorPos に失敗しました");
            finish_move(self_hwnd);
            return;
        }
        // 実際に動いたか確かめる。SetCursorPos は成功しても FALSE を返すことが
        // あるため、戻り値ではなく実測で判断する。
        // 「目標に届いていない」ではなく「元の位置から動いていない」で判定する。
        // 実測値は指示より少し遅れることがあるため。
        let mut after = std::mem::zeroed();
        if GetCursorPos(&mut after) != 0
            && ((x - from.x).abs() > 2 || (y - from.y).abs() > 2)
            && (after.x - from.x).abs() <= 2
            && (after.y - from.y).abs() <= 2
        {
            log_move_rejected((x, y), (after.x, after.y));
            finish_move(self_hwnd);
            return;
        }
        log::info(&format!(
            "移動しました [{}] → ({}, {}) ボタン=\"{}\"",
            via, x, y, label
        ));
        if play_sound {
            play_move_sound();
        }
        show_ripple(x, y);
        // 直前まで走っていた場合に備えて、差し替えたカーソルを戻す
        finish_move(self_hwnd);
        return;
    }

    // 進行方向と速度に応じてカーソルを選ぶ
    if cfg.cursor_animation {
        let speed = cursor::Speed::from_duration_ms(cfg.move_duration_ms);
        if !cursor::set_running(&cfg.theme, dx >= 0.0, speed) {
            log::debug(&format!(
                "カーソル: 差し替えできません theme=\"{}\"",
                cfg.theme
            ));
        }
    } else {
        log::debug("カーソル: 設定が無効なため差し替えません");
    }

    with_state(|s| {
        s.anim = Some(Anim {
            from: (from.x, from.y),
            to: (x, y),
            start: Instant::now(),
            duration: Duration::from_millis(cfg.move_duration_ms as u64),
            wobble: cfg.move_wobble as f64,
            last_set: (from.x, from.y),
            prev_set: (from.x, from.y),
            last_tick: Instant::now(),
            sound_pending: play_sound,
            stall_count: 0,
        });
    });

    log::info(&format!(
        "移動開始 [{}] ({}, {}) → ({}, {}) {}px を {}ms で ボタン=\"{}\"",
        via, from.x, from.y, x, y, distance as i32, cfg.move_duration_ms, label
    ));

    if !self_hwnd.is_null() {
        SetTimer(self_hwnd, TIMER_ANIM, ANIM_INTERVAL_MS, None);
    }
}

/// 走行アニメーションの 1 フレーム分を進める。
unsafe fn on_anim_tick(self_hwnd: HWND) {
    // 16ms ごとに走るので、必要な 1 項目だけ読む
    let Some((abort_on_user_move, threshold)) =
        with_state(|s| (s.cfg.abort_on_user_move, s.cfg.user_move_threshold as i32))
    else {
        return;
    };
    let Some(Some(anim)) = with_state(|s| s.anim) else {
        finish_move(self_hwnd);
        return;
    };

    // ユーザーが自分でマウスを動かしたら譲る。
    //
    // ただし処理が滞ってティックの間隔が開いた場合は判定しない。
    // UIA 走査などでメッセージループが数十 ms 止まると、その間の
    // わずかなマウスの動きがまとめて検出され、誤って中断してしまうため。
    let now = Instant::now();
    let stalled = now.duration_since(anim.last_tick) > TICK_STALL_LIMIT;
    with_state(|s| {
        if let Some(a) = s.anim.as_mut() {
            a.last_tick = now;
        }
    });

    if abort_on_user_move && !stalled {
        let mut now_pos = std::mem::zeroed();
        if GetCursorPos(&mut now_pos) != 0 {
            let dx = (now_pos.x - anim.last_set.0).abs();
            let dy = (now_pos.y - anim.last_set.1).abs();

            // カーソルの反映が 1 ティックぶん遅れることがある。
            // 昇格プロセスのウィンドウが前面のときに顕著で、そのままだと
            // 自分が動かしたぶんの差分をユーザー操作と誤判定してしまう。
            // 直前のティックで指示した移動量ぶんは許容する。
            let lag_x = (anim.last_set.0 - anim.prev_set.0).abs();
            let lag_y = (anim.last_set.1 - anim.prev_set.1).abs();

            // カーソルが開始地点から 1 度も動いていないなら、ユーザー操作では
            // なく SetCursorPos が拒否されている状態である。
            //
            // 拒否されている間もこちらの「指示」は目標へ進んでいくので、
            // last_set との差は開き続ける。それをユーザー操作として扱うと、
            // 管理者権限のウィンドウが前面にあるときの中断が、すべて
            // 「マウスが操作されました」と記録されてしまう（実際そうなっていた）。
            //
            // 拒否かどうかの判定は下の SetCursorPos の結果で行うため、
            // ここでは見送って先へ進める。
            let never_left_the_start =
                (now_pos.x - anim.from.0).abs() <= 2 && (now_pos.y - anim.from.1).abs() <= 2;

            if !never_left_the_start && (dx > threshold + lag_x || dy > threshold + lag_y) {
                // 実際の位置も残す。設定した座標のままなら SetCursorPos が
                // 効いておらず、まったく別の座標ならユーザーが動かしたと分かる
                log::info(&format!(
                    "中断: 走行中にマウスが操作されました 設定=({}, {}) 実際=({}, {}) 差=({}, {}) 開始地点=({}, {})",
                    anim.last_set.0,
                    anim.last_set.1,
                    now_pos.x,
                    now_pos.y,
                    now_pos.x - anim.last_set.0,
                    now_pos.y - anim.last_set.1,
                    anim.from.0,
                    anim.from.1
                ));
                finish_move(self_hwnd);
                return;
            }
        }
    } else if stalled {
        log::debug("走行: 処理が滞ったため今回の操作判定を見送ります");
    }

    let elapsed = Instant::now().duration_since(anim.start).as_secs_f64();
    let total = anim.duration.as_secs_f64().max(0.001);
    let t = (elapsed / total).clamp(0.0, 1.0);

    // smoothstep で加速→減速させる
    let eased = t * t * (3.0 - 2.0 * t);

    let fx = anim.from.0 as f64;
    let fy = anim.from.1 as f64;
    let tx = anim.to.0 as f64;
    let ty = anim.to.1 as f64;

    let mut px = fx + (tx - fx) * eased;
    let mut py = fy + (ty - fy) * eased;

    // 進行方向と垂直に小さく揺らして「走っている」感じを出す。
    // 揺れの周期は移動時間ではなく実時間に対して一定にする。こうしないと
    // 遅い設定のときだけ間延びして、カーソルの脚の動きと合わなくなる。
    // 終端では 0 に収束させ、狙った座標に必ず着地させる
    if anim.wobble > 0.0 && t < 1.0 {
        let dx = tx - fx;
        let dy = ty - fy;
        let len = (dx * dx + dy * dy).sqrt().max(1.0);
        let (nx, ny) = (-dy / len, dx / len);
        let decay = 1.0 - t;
        let swing = (elapsed * std::f64::consts::TAU * WOBBLE_HZ).sin() * anim.wobble * decay;
        px += nx * swing;
        py += ny * swing;
    }

    let (ix, iy) = (px.round() as i32, py.round() as i32);
    let accepted = SetCursorPos(ix, iy) != 0;

    // 戻り値だけでは動いたかどうか分からないので、毎回実測する。
    // SetCursorPos は成功しても FALSE を返すことがあり、逆に
    // TRUE を返しながらカーソルが動かないこともある。
    let mut actual = std::mem::zeroed();
    let measured = GetCursorPos(&mut actual) != 0;

    // 進んだかどうかは、戻り値ではなく実測だけで判断する。
    //
    // 戻り値は当てにならない。成功しても FALSE を返すことがあり、逆に
    // TRUE を返しながらカーソルが動かないこともある（前面が管理者権限で
    // 動いている場合に実際に観測された）。戻り値を条件にすると、
    // 後者のときに中断が働かず、動いていないのに最後まで走り切って
    // 「到着」の演出まで出てしまう。
    //
    // 「指示した点から離れている」ことは拒否の証拠にならない。カーソルの
    // 反映は 1 ティックぶん遅れることがあり、その差は 1 ティックの移動量
    // ぶんまで開く。見るべきは「直前に自分で設定した位置から動いていないか」で、
    // そのうえで 1 回では諦めない。
    let wanted_to_move = (ix - anim.last_set.0).abs() > 2 || (iy - anim.last_set.1).abs() > 2;
    let did_not_move =
        (actual.x - anim.last_set.0).abs() <= 2 && (actual.y - anim.last_set.1).abs() <= 2;
    let stuck = measured && wanted_to_move && did_not_move;

    let stalls = with_state(|s| {
        s.anim.as_mut().map_or(0, |a| {
            a.stall_count = if stuck {
                a.stall_count.saturating_add(1)
            } else {
                0
            };
            a.stall_count
        })
    })
    .unwrap_or(0);

    if stalls >= MAX_STALLED_TICKS {
        log_move_rejected((ix, iy), (actual.x, actual.y));
        finish_move(self_hwnd);
        return;
    }
    if !accepted {
        log::debug(&format!(
            "SetCursorPos が FALSE を返しました ({}, {}) 実際=({}, {}) 進まなかった回数={}",
            ix, iy, actual.x, actual.y, stalls
        ));
    }

    // 音は「カーソルが実際に動いた」ことを確かめてから鳴らす。
    //
    // 判定は開始地点から離れたかどうかで行う。指示した座標との一致では
    // 足りない。最初のティックは指示座標が開始地点とほぼ同じなので、
    // 一致していても動いた証拠にならないからである。
    // SetCursorPos の戻り値も根拠にならない（上のとおり当てにならない）。
    //
    // ここを緩めると、UIPI で拒否される環境——管理者権限のウィンドウが
    // 前面にあるときなど——で「鳴き声だけ出てカーソルは動かない」ことになる。
    let moved_from_start =
        measured && ((actual.x - anim.from.0).abs() > 2 || (actual.y - anim.from.1).abs() > 2);
    if moved_from_start {
        let should_play = with_state(|s| {
            s.anim
                .as_mut()
                .map(|a| std::mem::replace(&mut a.sound_pending, false))
                .unwrap_or(false)
        })
        .unwrap_or(false);
        if should_play {
            play_move_sound();
        }
    }

    with_state(|s| {
        if let Some(a) = s.anim.as_mut() {
            a.prev_set = a.last_set;
            a.last_set = (ix, iy);
        }
    });

    if t >= 1.0 {
        SetCursorPos(anim.to.0, anim.to.1);

        // 「到着」と演出は、本当に着いたことを確かめてから。
        //
        // 到達演出はこちらが自前のウィンドウを描くだけなので、カーソルを
        // 動かせない状況でも問題なく表示できてしまう。確認せずに出すと、
        // カーソルは元の場所にあるのに着地点だけ光る、という嘘になる。
        let mut landed = std::mem::zeroed();
        let arrived = GetCursorPos(&mut landed) != 0
            && (landed.x - anim.to.0).abs() <= 2
            && (landed.y - anim.to.1).abs() <= 2;

        if arrived {
            log::info(&format!("到着しました → ({}, {})", anim.to.0, anim.to.1));
            show_ripple(anim.to.0, anim.to.1);
        } else {
            log_move_rejected((anim.to.0, anim.to.1), (landed.x, landed.y));
        }
        finish_move(self_hwnd);
    }
}

/// 移動が確定したときの音を鳴らす。
///
/// 呼び出し側で `sound_enabled` は判定済みとする。
unsafe fn play_move_sound() {
    let Some(path) = with_state(|s| s.cfg.effective_wav()) else {
        return;
    };
    if sound::play_wav(&path) {
        log::debug(&format!("サウンド: 再生しました path=\"{}\"", path));
    } else {
        log::info(&format!("警告: wav を再生できません path=\"{}\"", path));
    }
}

/// 到達地点に同心円のアニメーションを表示する。
unsafe fn show_ripple(x: i32, y: i32) {
    let Some((enabled, size, duration, color)) = with_state(|s| {
        (
            s.cfg.ripple_enabled,
            s.cfg.ripple_size,
            s.cfg.ripple_duration_ms,
            s.cfg.ripple_color,
        )
    }) else {
        return;
    };
    if enabled {
        ripple::play(x, y, size, duration, color);
    }
}

/// アニメーションを終了し、カーソルを元に戻す。
unsafe fn finish_move(self_hwnd: HWND) {
    KillTimer(self_hwnd, TIMER_ANIM);
    with_state(|s| s.anim = None);
    cursor::restore();
}

// ---- トレイメニュー -----------------------------------------------------

unsafe fn on_tray_click(hwnd: HWND) {
    // 入れ子の呼び出しは無視する（詳細は IN_TRAY_CLICK のコメント）
    if IN_TRAY_CLICK.with(|c| c.replace(true)) {
        log::debug("トレイ: メニュー処理中の再入を無視しました");
        return;
    }
    handle_tray_command(hwnd);
    IN_TRAY_CLICK.with(|c| c.set(false));
}

unsafe fn handle_tray_command(hwnd: HWND) {
    // TrackPopupMenu はメッセージをポンプするため、ロックを保持したまま呼ばない
    let Some(cfg) = with_state(|s| s.cfg.clone()) else {
        return;
    };
    let cmd = tray::show_menu(hwnd, &cfg);
    if cmd == 0 {
        return;
    }

    let mut cfg = cfg;
    // 手で config.ini を編集した後にメニューを操作した場合、
    // メモリ上の古い設定で上書きしてしまわないよう読み直す
    if config::file_changed_externally() {
        log::info("設定ファイルが外部から変更されていたため読み直しました");
        cfg = Config::load();
        lang::load(&cfg.language);
        log::set_level(cfg.log_level);
        WATCH_TITLES.store(cfg.watch_title_changes, Ordering::Relaxed);
        WATCH_FOCUS.store(cfg.watch_focus_changes, Ordering::Relaxed);
    }

    let mut changed = true;
    let mut save = true;

    match cmd {
        tray::CMD_TOGGLE_ENABLED => cfg.enabled = !cfg.enabled,
        tray::CMD_TOGGLE_SOUND => cfg.sound_enabled = !cfg.sound_enabled,
        tray::CMD_TOGGLE_MOVE_ANIM => cfg.move_animation = !cfg.move_animation,
        tray::CMD_TOGGLE_RIPPLE => cfg.ripple_enabled = !cfg.ripple_enabled,
        c if c >= tray::CMD_THEME_BASE
            && ((c - tray::CMD_THEME_BASE) as usize) < tray::MAX_THEMES =>
        {
            let index = (c - tray::CMD_THEME_BASE) as usize;
            let selected = if index == 0 {
                // 先頭は「既定」。テーマを使わない
                String::new()
            } else {
                let list = theme::available();
                match list.get(index - 1) {
                    Some(name) => name.clone(),
                    None => return,
                }
            };

            // テーマを実際に切り替えたときだけ、カーソル差し替えの設定を合わせる。
            //
            // 既定に戻せば Windows のカーソルを使いたいはずで、テーマを選んだなら
            // そのカーソルを見たいはず、という想定。ここで一度だけ設定するので、
            // この後メニューから手動で切り替えた分はそのまま残る。
            if selected != cfg.theme {
                cfg.cursor_animation = !selected.is_empty();
            }
            cfg.theme = selected;

            // 差し替え中のカーソルを元に戻す。次回から新しいテーマが使われる
            cursor::restore();
        }
        tray::CMD_TOGGLE_SKIP_FILE => cfg.skip_file_dialogs = !cfg.skip_file_dialogs,
        tray::CMD_TOGGLE_AUTOSTART => {
            toggle_autostart();
            changed = false;
            save = false;
        }
        tray::CMD_TOGGLE_CURSOR_ANIM => {
            cfg.cursor_animation = !cfg.cursor_animation;
            if !cfg.cursor_animation {
                cursor::restore();
            }
        }
        tray::CMD_CHOOSE_WAV => {
            // ファイル選択ダイアログ自体にカーソルを持っていかれると邪魔なので抑止する
            with_state(|s| s.suppress_own += 1);
            let picked = tray::choose_wav(hwnd, &cfg.wav_path);
            with_state(|s| s.suppress_own = s.suppress_own.saturating_sub(1));
            match picked {
                Some(path) => {
                    // 選んだファイルそのものを試聴させる（テーマより優先される指定）
                    cfg.wav_path = path;
                    sound::play_wav(&cfg.wav_path);
                }
                None => {
                    changed = false;
                    save = false;
                }
            }
        }
        tray::CMD_TEST_SOUND => {
            // 実際にダイアログ検出時に鳴る音を確認するためのものなので、
            // テーマの解決を通した結果を鳴らす
            if !sound::play_wav(&cfg.effective_wav()) {
                info_box(&lang::t("msg.wav_failed"));
            }
            changed = false;
            save = false;
        }
        tray::CMD_TEST_DIALOG => {
            // メニューを閉じてから表示させたいので自分に投げ直す
            PostMessageW(hwnd, WM_APP_TEST, 0, 0);
            changed = false;
            save = false;
        }
        tray::CMD_OPEN_LOG => {
            let path = config::log_path();
            if !path.exists() {
                // 開きっぱなしのハンドルがある状態で外から書くと、
                // ハンドルが持つ書き込み位置と実体がずれる。先に手放しておく。
                log::close();
                let _ = std::fs::create_dir_all(path.parent().unwrap_or(&path));
                // BOM は format! の外で連結する。
                // format! の中では {{ }} がエスケープとして解釈され、
                // \u{feff} のような Unicode エスケープが壊れてしまう。
                let text = format!("\u{feff}{}\r\n", lang::t("msg.log_empty"));
                let _ = std::fs::write(&path, text);
            }
            tray::open_in_editor(&path);
            changed = false;
            save = false;
        }
        tray::CMD_OPEN_CONFIG => {
            tray::open_config_in_editor();
            changed = false;
            save = false;
        }
        tray::CMD_RELOAD_CONFIG => {
            cfg = Config::load();
            lang::load(&cfg.language);
            log::set_level(cfg.log_level);
            WATCH_TITLES.store(cfg.watch_title_changes, Ordering::Relaxed);
            WATCH_FOCUS.store(cfg.watch_focus_changes, Ordering::Relaxed);
            log::info("設定を再読み込みしました");
            save = false;
        }
        tray::CMD_ABOUT => {
            // バージョンは Cargo.toml から取る。言語ファイルに書くと更新漏れが起きる
            info_box(&lang::tf(
                "msg.about",
                &[
                    env!("CARGO_PKG_VERSION"),
                    &config::config_dir().display().to_string(),
                ],
            ));
            changed = false;
            save = false;
        }
        tray::CMD_EXIT => {
            DestroyWindow(hwnd);
            return;
        }
        c if c >= tray::CMD_SPEED_BASE
            && ((c - tray::CMD_SPEED_BASE) as usize) < tray::SPEED_PRESETS.len() =>
        {
            let (_, ms) = tray::SPEED_PRESETS[(c - tray::CMD_SPEED_BASE) as usize];
            cfg.move_duration_ms = ms;
        }
        c if c >= tray::CMD_LANG_BASE
            && ((c - tray::CMD_LANG_BASE) as usize) < tray::MAX_LANGUAGES =>
        {
            let list = lang::available();
            // 一覧の範囲外を選んだ場合は何もせずに抜ける。
            // ここで return するため、changed / save への代入は不要。
            let Some((code, _)) = list.get((c - tray::CMD_LANG_BASE) as usize) else {
                return;
            };
            cfg.language = code.clone();
            lang::load(&cfg.language);
        }
        c if (tray::CMD_LOG_BASE..=tray::CMD_LOG_BASE + 2).contains(&c) => {
            cfg.log_level = c - tray::CMD_LOG_BASE;
            log::set_level(cfg.log_level);
        }
        c if c >= tray::CMD_DELAY_BASE
            && ((c - tray::CMD_DELAY_BASE) as usize) < tray::DELAY_PRESETS.len() =>
        {
            cfg.delay_ms = tray::DELAY_PRESETS[(c - tray::CMD_DELAY_BASE) as usize];
        }
        _ => {
            changed = false;
            save = false;
        }
    }

    if changed {
        if save {
            if let Err(e) = cfg.save() {
                // 保存できないと、メニューでの変更が再起動で消える。
                // 黙って失敗すると原因の見当がつかない
                log::info(&format!(
                    "設定を保存できませんでした: {e} path=\"{}\"",
                    config::config_path().display()
                ));
            }
        }
        with_state(|s| s.cfg = cfg.clone());
        tray::update_icon(hwnd, WM_APP_TRAY, &cfg);
    }
}
