use std::cell::Cell;
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{HWND, POINT};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::Dialogs::{GetOpenFileNameW, OPENFILENAMEW};
use windows_sys::Win32::UI::Shell::{
    ShellExecuteW, Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE,
    NIM_MODIFY, NOTIFYICONDATAW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyIcon, DestroyMenu, GetCursorPos, GetSystemMetrics,
    LoadIconW, LoadImageW, PostMessageW, SetForegroundWindow, TrackPopupMenu, HICON, HMENU,
    IDI_APPLICATION, IMAGE_ICON, LR_DEFAULTCOLOR, LR_LOADFROMFILE, MF_CHECKED, MF_GRAYED, MF_POPUP,
    MF_SEPARATOR, MF_STRING, SM_CXSMICON, SM_CYSMICON, TPM_NONOTIFY, TPM_RETURNCMD,
    TPM_RIGHTBUTTON, WM_NULL,
};

use crate::config::Config;
use crate::lang::{t, tf};
use crate::util::{fill_wide, from_wide, system32_path, wide};

thread_local! {
    /// 生成済みアイコンのキャッシュ（0 = 未取得）
    static ICON_CACHE: Cell<usize> = const { Cell::new(0) };
    /// キャッシュしたアイコンのテーマ名
    static ICON_THEME: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
    /// キャッシュしたアイコンを破棄してよいか。
    /// システム標準アイコンは共有ハンドルなので破棄してはいけない。
    static ICON_OWNED: Cell<bool> = const { Cell::new(false) };
}

pub const TRAY_UID: u32 = 1;

// メニューコマンド ID
pub const CMD_TOGGLE_ENABLED: u32 = 1001;
pub const CMD_TOGGLE_SOUND: u32 = 1002;
pub const CMD_CHOOSE_WAV: u32 = 1003;
pub const CMD_TEST_SOUND: u32 = 1004;
pub const CMD_OPEN_CONFIG: u32 = 1005;
pub const CMD_RELOAD_CONFIG: u32 = 1006;
pub const CMD_ABOUT: u32 = 1007;
pub const CMD_TEST_DIALOG: u32 = 1008;
pub const CMD_OPEN_LOG: u32 = 1009;
pub const CMD_TOGGLE_MOVE_ANIM: u32 = 1010;
pub const CMD_TOGGLE_CURSOR_ANIM: u32 = 1011;
pub const CMD_TOGGLE_SKIP_FILE: u32 = 1012;
pub const CMD_TOGGLE_AUTOSTART: u32 = 1013;
pub const CMD_TOGGLE_RIPPLE: u32 = 1014;
pub const CMD_EXIT: u32 = 1099;
/// 遅延プリセット: CMD_DELAY_BASE + index
pub const CMD_DELAY_BASE: u32 = 1100;
/// ログレベル: CMD_LOG_BASE + level (0..=2)
pub const CMD_LOG_BASE: u32 = 1200;
/// 走行速度: CMD_SPEED_BASE + index
pub const CMD_SPEED_BASE: u32 = 1400;
/// 言語: CMD_LANG_BASE + index
pub const CMD_LANG_BASE: u32 = 1500;
/// メニューに並べる言語の上限
pub const MAX_LANGUAGES: usize = 32;
/// テーマ選択: CMD_THEME_BASE + index（0 = 既定、1 以降が theme 配下）
///
/// 範囲が他のプリセットと重ならないよう、末尾に十分な間隔を空けて置く。
pub const CMD_THEME_BASE: u32 = 1600;
pub const MAX_THEMES: usize = 33;

/// 走行速度のプリセット（言語キー, move_duration_ms）
pub const SPEED_PRESETS: &[(&str, u32)] = &[
    ("menu.speed.fast", 160),
    ("menu.speed.normal", 320),
    ("menu.speed.slow", 640),
];

pub const DELAY_PRESETS: &[u32] = &[0, 100, 200, 300, 500, 800, 1000, 2000];

/// アプリアイコンを取得する。
///
/// 1. exe に埋め込まれたリソース (build.rs で埋め込む、ID=1)
/// 2. exe と同じフォルダの icon.ico
/// 3. システム標準アイコン
///
/// 生成した HICON はキャッシュする。更新のたびに作り直すとハンドルリークになるため。
unsafe fn app_icon(theme: &str) -> HICON {
    // テーマが変わったらキャッシュを捨てて読み直す
    let same_theme = ICON_THEME.with(|c| c.borrow().as_str() == theme);
    let cached = ICON_CACHE.with(|c| c.get());
    if cached != 0 && same_theme {
        return cached as HICON;
    }

    let cx = GetSystemMetrics(SM_CXSMICON);
    let cy = GetSystemMetrics(SM_CYSMICON);
    let hinst = GetModuleHandleW(std::ptr::null());

    // 1) 埋め込みリソース
    //
    // MAKEINTRESOURCEW(1) は「ポインタではなくリソース ID」を表す約束事で、
    // 上位ワードが 0 の値を Win32 が整数 ID として解釈する。
    // dangling ポインタを作っているように見えるが、参照されることはない。
    #[allow(clippy::manual_dangling_ptr)]
    let resource_id = 1 as *const u16;

    // テーマが指定されていればそちらを先に試す
    let mut icon: HICON = null_mut();
    if theme.is_empty() {
        icon = LoadImageW(hinst, resource_id, IMAGE_ICON, cx, cy, LR_DEFAULTCOLOR) as HICON;
    }

    // 2) テーマの icon.ico
    if icon.is_null() {
        if let Some(path) = crate::theme::icon(theme) {
            let w = wide(&path.to_string_lossy());
            icon = LoadImageW(
                null_mut(),
                w.as_ptr(),
                IMAGE_ICON,
                cx,
                cy,
                LR_LOADFROMFILE | LR_DEFAULTCOLOR,
            ) as HICON;
        }
    }

    // 3) 埋め込みリソース（テーマ指定時のフォールバック）
    if icon.is_null() {
        icon = LoadImageW(hinst, resource_id, IMAGE_ICON, cx, cy, LR_DEFAULTCOLOR) as HICON;
    }

    // ここまでで得たハンドルは自分で作ったもので、破棄する責任がある
    let mut owned = !icon.is_null();

    // 4) 標準アイコン。共有ハンドルなので破棄してはいけない
    if icon.is_null() {
        icon = LoadIconW(null_mut(), IDI_APPLICATION);
        owned = false;
    }

    // 新しいアイコンを用意してから、古いものを破棄する。
    // テーマを切り替えるたびに捨てているとハンドルリークになる一方、
    // 先に破棄するとキャッシュに一瞬でも無効なハンドルが残ってしまう。
    let old = ICON_CACHE.with(|c| c.replace(icon as usize));
    let old_owned = ICON_OWNED.with(|c| c.replace(owned));
    if old != 0 && old != icon as usize && old_owned {
        DestroyIcon(old as HICON);
    }
    ICON_THEME.with(|c| *c.borrow_mut() = theme.to_string());

    icon
}

unsafe fn build_nid(hwnd: HWND, callback_msg: u32, cfg: Option<&Config>) -> NOTIFYICONDATAW {
    let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
    nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = TRAY_UID;
    nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
    nid.uCallbackMessage = callback_msg;
    nid.hIcon = app_icon(cfg.map(|c| c.theme.as_str()).unwrap_or(""));
    let tip = match cfg {
        Some(c) => tf(
            "tip.status",
            &[
                &t(if c.enabled { "tip.on" } else { "tip.off" }),
                &c.delay_ms.to_string(),
                &t(if c.sound_enabled { "tip.on" } else { "tip.off" }),
            ],
        ),
        None => "DialogCursorMover".to_string(),
    };
    fill_wide(&mut nid.szTip, &tip);
    nid
}

pub unsafe fn add_icon(hwnd: HWND, callback_msg: u32, cfg: &Config) -> bool {
    let nid = build_nid(hwnd, callback_msg, Some(cfg));
    Shell_NotifyIconW(NIM_ADD, &nid) != 0
}

pub unsafe fn update_icon(hwnd: HWND, callback_msg: u32, cfg: &Config) {
    let nid = build_nid(hwnd, callback_msg, Some(cfg));
    Shell_NotifyIconW(NIM_MODIFY, &nid);
}

pub unsafe fn remove_icon(hwnd: HWND) {
    let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
    nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = TRAY_UID;
    Shell_NotifyIconW(NIM_DELETE, &nid);
}

/// サブメニューを作る。失敗したら None を返す。
///
/// `CreatePopupMenu` はリソース枯渇時に null を返す。そのまま
/// `AppendMenuW` へ渡すとその項目が壊れるため、呼び出し側で分岐する。
unsafe fn sub_menu() -> Option<HMENU> {
    let m = CreatePopupMenu();
    if m.is_null() {
        crate::log::info("サブメニューを作成できませんでした");
        None
    } else {
        Some(m)
    }
}

/// コンテキストメニューを表示し、選択されたコマンド ID を返す（0 = 未選択）。
pub unsafe fn show_menu(hwnd: HWND, cfg: &Config) -> u32 {
    let menu = CreatePopupMenu();
    if menu.is_null() {
        return 0;
    }

    let item = |text: &str| wide(text);

    // 有効／無効
    let s = item(&t("menu.enabled"));
    AppendMenuW(
        menu,
        MF_STRING | if cfg.enabled { MF_CHECKED } else { 0 },
        CMD_TOGGLE_ENABLED as usize,
        s.as_ptr(),
    );

    let s = item(&t("menu.skip_file_dialogs"));
    AppendMenuW(
        menu,
        MF_STRING | if cfg.skip_file_dialogs { MF_CHECKED } else { 0 },
        CMD_TOGGLE_SKIP_FILE as usize,
        s.as_ptr(),
    );

    AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());

    // 遅延サブメニュー
    if let Some(delay_menu) = sub_menu() {
        let cur = item(&tf("menu.delay.current", &[&cfg.delay_ms.to_string()]));
        AppendMenuW(delay_menu, MF_STRING | MF_GRAYED, 0, cur.as_ptr());
        AppendMenuW(delay_menu, MF_SEPARATOR, 0, std::ptr::null());
        for (i, ms) in DELAY_PRESETS.iter().enumerate() {
            let label = item(&format!("{} ms", ms));
            AppendMenuW(
                delay_menu,
                MF_STRING | if *ms == cfg.delay_ms { MF_CHECKED } else { 0 },
                (CMD_DELAY_BASE + i as u32) as usize,
                label.as_ptr(),
            );
        }
        let s = item(&t("menu.delay"));
        // 親メニューに取り付けられなければ、DestroyMenu(menu) では解放されない。
        // USER オブジェクトが枯渇しかけている状況で漏らすと事態を悪化させる
        if AppendMenuW(menu, MF_POPUP, delay_menu as usize, s.as_ptr()) == 0 {
            DestroyMenu(delay_menu);
        }
    }

    AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());

    // サウンド
    let s = item(&t("menu.sound"));
    AppendMenuW(
        menu,
        MF_STRING | if cfg.sound_enabled { MF_CHECKED } else { 0 },
        CMD_TOGGLE_SOUND as usize,
        s.as_ptr(),
    );

    // 再生時と同じ解決を通す。cfg.wav_path を直接見ると、
    // テーマの音が鳴っているのに既定のファイル名が表示されてしまう
    let effective = cfg.effective_wav();
    let name = std::path::Path::new(&effective)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| t("msg.sound_unset"));
    let s = item(&tf("menu.sound.current", &[&name]));
    AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, s.as_ptr());

    let s = item(&t("menu.sound.choose"));
    AppendMenuW(menu, MF_STRING, CMD_CHOOSE_WAV as usize, s.as_ptr());

    let s = item(&t("menu.sound.test"));
    AppendMenuW(menu, MF_STRING, CMD_TEST_SOUND as usize, s.as_ptr());

    AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());

    // 演出
    let s = item(&t("menu.move_animation"));
    AppendMenuW(
        menu,
        MF_STRING | if cfg.move_animation { MF_CHECKED } else { 0 },
        CMD_TOGGLE_MOVE_ANIM as usize,
        s.as_ptr(),
    );

    // 走る速度
    if let Some(speed_menu) = sub_menu() {
        let cur = item(&tf(
            "menu.delay.current",
            &[&cfg.move_duration_ms.to_string()],
        ));
        AppendMenuW(speed_menu, MF_STRING | MF_GRAYED, 0, cur.as_ptr());
        AppendMenuW(speed_menu, MF_SEPARATOR, 0, std::ptr::null());
        for (i, (label, ms)) in SPEED_PRESETS.iter().enumerate() {
            let s = item(&tf("menu.speed.item", &[&t(label), &ms.to_string()]));
            AppendMenuW(
                speed_menu,
                MF_STRING
                    | if *ms == cfg.move_duration_ms {
                        MF_CHECKED
                    } else {
                        0
                    },
                (CMD_SPEED_BASE + i as u32) as usize,
                s.as_ptr(),
            );
        }
        let s = item(&t("menu.move_speed"));
        // 親メニューに取り付けられなければ、DestroyMenu(menu) では解放されない。
        // USER オブジェクトが枯渇しかけている状況で漏らすと事態を悪化させる
        if AppendMenuW(
            menu,
            MF_POPUP | if cfg.move_animation { 0 } else { MF_GRAYED },
            speed_menu as usize,
            s.as_ptr(),
        ) == 0
        {
            DestroyMenu(speed_menu);
        }
    }

    // テーマ（アイコン・カーソル・音の差し替え）
    if let Some(theme_menu) = sub_menu() {
        let s = item(&t("menu.theme.default"));
        AppendMenuW(
            theme_menu,
            MF_STRING | if cfg.theme.is_empty() { MF_CHECKED } else { 0 },
            CMD_THEME_BASE as usize,
            s.as_ptr(),
        );
        for (i, name) in crate::theme::available().iter().enumerate() {
            if i + 1 >= MAX_THEMES {
                break;
            }
            let s = item(name);
            AppendMenuW(
                theme_menu,
                MF_STRING | if cfg.theme == *name { MF_CHECKED } else { 0 },
                (CMD_THEME_BASE + i as u32 + 1) as usize,
                s.as_ptr(),
            );
        }
        let s = item(&t("menu.theme"));
        // 親メニューに取り付けられなければ、DestroyMenu(menu) では解放されない。
        // USER オブジェクトが枯渇しかけている状況で漏らすと事態を悪化させる
        if AppendMenuW(menu, MF_POPUP, theme_menu as usize, s.as_ptr()) == 0 {
            DestroyMenu(theme_menu);
        }
    }

    let s = item(&t("menu.ripple"));
    AppendMenuW(
        menu,
        MF_STRING | if cfg.ripple_enabled { MF_CHECKED } else { 0 },
        CMD_TOGGLE_RIPPLE as usize,
        s.as_ptr(),
    );

    let s = item(&t("menu.cursor_animation"));
    AppendMenuW(
        menu,
        MF_STRING | if cfg.cursor_animation { MF_CHECKED } else { 0 },
        CMD_TOGGLE_CURSOR_ANIM as usize,
        s.as_ptr(),
    );

    AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());

    // 自動起動（タスクスケジューラへの登録）
    let registered = crate::autostart::is_registered();
    let s = item(&t("menu.autostart"));
    AppendMenuW(
        menu,
        MF_STRING | if registered { MF_CHECKED } else { 0 },
        CMD_TOGGLE_AUTOSTART as usize,
        s.as_ptr(),
    );

    AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());

    // 診断
    if let Some(diag) = sub_menu() {
        let s = item(&t("menu.test_dialog"));
        AppendMenuW(diag, MF_STRING, CMD_TEST_DIALOG as usize, s.as_ptr());
        AppendMenuW(diag, MF_SEPARATOR, 0, std::ptr::null());
        for (level, key) in [
            (0u32, "menu.log.off"),
            (1, "menu.log.normal"),
            (2, "menu.log.verbose"),
        ] {
            let s = item(&t(key));
            AppendMenuW(
                diag,
                MF_STRING
                    | if cfg.log_level == level {
                        MF_CHECKED
                    } else {
                        0
                    },
                (CMD_LOG_BASE + level) as usize,
                s.as_ptr(),
            );
        }
        AppendMenuW(diag, MF_SEPARATOR, 0, std::ptr::null());
        let s = item(&t("menu.open_log"));
        AppendMenuW(diag, MF_STRING, CMD_OPEN_LOG as usize, s.as_ptr());
        let s = item(&t("menu.diagnostics"));
        // 親メニューに取り付けられなければ、DestroyMenu(menu) では解放されない。
        // USER オブジェクトが枯渇しかけている状況で漏らすと事態を悪化させる
        if AppendMenuW(menu, MF_POPUP, diag as usize, s.as_ptr()) == 0 {
            DestroyMenu(diag);
        }
    }

    // 言語（lang ディレクトリに 2 つ以上ある場合のみ出す）
    let languages = crate::lang::available();
    if languages.len() > 1 {
        if let Some(lang_menu) = sub_menu() {
            for (i, (code, name)) in languages.iter().take(MAX_LANGUAGES).enumerate() {
                let s = item(name);
                AppendMenuW(
                    lang_menu,
                    // 読み込み側は言語コードを小文字化するので、
                    // 大小文字を無視して比較しないと zh-TW のような表記で
                    // チェックが付かなくなる
                    MF_STRING
                        | if code.eq_ignore_ascii_case(&cfg.language) {
                            MF_CHECKED
                        } else {
                            0
                        },
                    (CMD_LANG_BASE + i as u32) as usize,
                    s.as_ptr(),
                );
            }
            let s = item(&t("menu.language"));
            // 親メニューに取り付けられなければ、DestroyMenu(menu) では解放されない。
            // USER オブジェクトが枯渇しかけている状況で漏らすと事態を悪化させる
            if AppendMenuW(menu, MF_POPUP, lang_menu as usize, s.as_ptr()) == 0 {
                DestroyMenu(lang_menu);
            }
        }
    }

    let s = item(&t("menu.open_config"));
    AppendMenuW(menu, MF_STRING, CMD_OPEN_CONFIG as usize, s.as_ptr());
    let s = item(&t("menu.reload_config"));
    AppendMenuW(menu, MF_STRING, CMD_RELOAD_CONFIG as usize, s.as_ptr());
    let s = item(&t("menu.about"));
    AppendMenuW(menu, MF_STRING, CMD_ABOUT as usize, s.as_ptr());

    AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());

    let s = item(&t("menu.exit"));
    AppendMenuW(menu, MF_STRING, CMD_EXIT as usize, s.as_ptr());

    let mut pt: POINT = std::mem::zeroed();
    GetCursorPos(&mut pt);

    // トレイメニューの作法: 先にフォアグラウンド化し、閉じた後にダミーメッセージを送る
    SetForegroundWindow(hwnd);
    let cmd = TrackPopupMenu(
        menu,
        TPM_RIGHTBUTTON | TPM_RETURNCMD | TPM_NONOTIFY,
        pt.x,
        pt.y,
        0,
        hwnd,
        std::ptr::null(),
    );
    PostMessageW(hwnd, WM_NULL, 0, 0);
    DestroyMenu(menu);

    cmd as u32
}

const OFN_FILEMUSTEXIST: u32 = 0x0000_1000;
const OFN_PATHMUSTEXIST: u32 = 0x0000_0800;
const OFN_EXPLORER: u32 = 0x0008_0000;
const OFN_NOCHANGEDIR: u32 = 0x0000_0008;

/// .wav 選択ダイアログ。キャンセル時は None。
pub unsafe fn choose_wav(owner: HWND, current: &str) -> Option<String> {
    let mut buf: Vec<u16> = vec![0u16; 1024];
    let cur = wide(current);
    if cur.len() <= buf.len() {
        buf[..cur.len()].copy_from_slice(&cur);
    }

    let mut filter: Vec<u16> = Vec::new();
    for part in [
        t("msg.filter_wav"),
        "*.wav".to_string(),
        t("msg.filter_all"),
        "*.*".to_string(),
    ] {
        filter.extend(part.encode_utf16());
        filter.push(0);
    }
    filter.push(0);

    let title = wide(&t("msg.choose_wav"));

    let mut ofn: OPENFILENAMEW = std::mem::zeroed();
    ofn.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
    ofn.hwndOwner = owner;
    ofn.lpstrFilter = filter.as_ptr();
    ofn.nFilterIndex = 1;
    ofn.lpstrFile = buf.as_mut_ptr();
    ofn.nMaxFile = buf.len() as u32;
    ofn.lpstrTitle = title.as_ptr();
    ofn.Flags = OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_EXPLORER | OFN_NOCHANGEDIR;

    if GetOpenFileNameW(&mut ofn) != 0 {
        let s = from_wide(&buf);
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    } else {
        None
    }
}

/// 指定したファイルをメモ帳で開く。
pub unsafe fn open_in_editor(path: &std::path::Path) {
    let verb = wide("open");
    // 絶対パスで起動する。修飾されていない名前だと、シェルがシステム
    // ディレクトリより先にカレントディレクトリを探してしまう。
    let file = wide(&system32_path("notepad.exe"));
    // パスは必ず引用符で囲む。囲まないと空白が区切りとして解釈され、
    // `C:\Users\Taro Yamada\...` のようなパスで別のファイル名になってしまう。
    // 設定・ログは %APPDATA% 配下へ退避することがあるため、実際に起こりうる。
    // Windows のファイル名に `"` は使えないので、エスケープは不要。
    let args = wide(&format!("\"{}\"", path.to_string_lossy()));
    ShellExecuteW(
        null_mut(),
        verb.as_ptr(),
        file.as_ptr(),
        args.as_ptr(),
        std::ptr::null(),
        1, // SW_SHOWNORMAL
    );
}

/// 設定ファイルをメモ帳で開く。
pub unsafe fn open_config_in_editor() {
    open_in_editor(&crate::config::config_path());
}
