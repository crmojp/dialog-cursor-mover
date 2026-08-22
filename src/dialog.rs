use windows_sys::Win32::Foundation::{CloseHandle, BOOL, HWND, LPARAM, POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromPoint, MonitorFromWindow, MONITORINFO,
};
use windows_sys::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, GetClassNameW, GetWindow, GetWindowRect, GetWindowTextW,
    GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible, GWL_ID, GWL_STYLE,
};

use crate::util::from_wide;

pub const IDOK: i32 = 1;

const WS_CHILD: u32 = 0x4000_0000;
const WS_POPUP: u32 = 0x8000_0000;
const BS_TYPEMASK: u32 = 0x0000_000F;
const BS_PUSHBUTTON: u32 = 0x0000_0000;
const BS_DEFPUSHBUTTON: u32 = 0x0000_0001;
/// オーナードローボタン。CPU-Z のように自前描画のボタンで使われる
const BS_OWNERDRAW: u32 = 0x0000_000B;
const WS_THICKFRAME: u32 = 0x0004_0000;
const WS_CAPTION: u32 = 0x00C0_0000;
const GW_OWNER: u32 = 4;
const MONITOR_DEFAULTTONEAREST: u32 = 2;
/// どのモニタにも乗っていなければ NULL を返す
const MONITOR_DEFAULTTONULL: u32 = 0;

/// ウィンドウが載っているモニタの表示領域サイズ。
///
/// `GetSystemMetrics(SM_CXSCREEN)` はプライマリモニタしか返さないため、
/// マルチモニタ環境ではセカンダリ上のウィンドウを誤判定してしまう。
unsafe fn monitor_size_of(hwnd: HWND) -> Option<(i32, i32)> {
    let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
    if monitor.is_null() {
        return None;
    }
    let mut info: MONITORINFO = std::mem::zeroed();
    info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
    if GetMonitorInfoW(monitor, &mut info) == 0 {
        return None;
    }
    let w = info.rcMonitor.right - info.rcMonitor.left;
    let h = info.rcMonitor.bottom - info.rcMonitor.top;
    if w <= 0 || h <= 0 {
        None
    } else {
        Some((w, h))
    }
}

/// このウィンドウの所有者（オーナー）を返す。
pub unsafe fn owner_of(hwnd: HWND) -> HWND {
    GetWindow(hwnd, GW_OWNER)
}

/// 表題を持つが、ダイアログではないことが明らかなウィンドウクラス。
///
/// ドラッグ中のゴースト画像やポップアップの器などが該当する。
/// これらは一瞬だけ現れては消えるため、走査しても無駄になる。
const NEVER_DIALOG_CLASSES: &[&str] = &[
    "SysDragImage",
    "SysShadow",
    "tooltips_class32",
    "DragVisualWindow",
    "PopupWindowSiteBridge",
    "TopLevelWindowForOverflowXamlIsland",
];

/// 「ダイアログらしい」ウィンドウかを、スタイルとサイズから判定する。
///
/// UIA 走査は重いので、ブラウザやエクスプローラのような通常のアプリ
/// ウィンドウを対象から外すために使う。
///
/// 最も効く手がかりは「オーナーウィンドウを持つか」で、ダイアログは基本的に
/// 呼び出し元に所有されるのに対し、アプリのメインウィンドウは所有者を持たない。
/// タイトルバーに最小化・最大化ボタンを持つダイアログ（エクスプローラの
/// 「ファイルの置換またはスキップ」など）もあるため、ボタンの有無だけで
/// 判定してはいけない。
pub unsafe fn is_dialog_like(hwnd: HWND) -> bool {
    if !is_top_level(hwnd) || IsWindowVisible(hwnd) == 0 {
        return false;
    }

    // 明らかにダイアログではないものを先に落とす
    if NEVER_DIALOG_CLASSES.iter().any(|c| class_contains(hwnd, c)) {
        return false;
    }

    let Some(r) = window_rect(hwnd) else {
        return false;
    };
    let (w, h) = (r.right - r.left, r.bottom - r.top);
    // そのウィンドウが載っているモニタを基準にする
    let Some((sw, sh)) = monitor_size_of(hwnd) else {
        return true;
    };

    // タスクバーのような「画面いっぱい」のウィンドウは常に除外する
    if w * 10 > sw * 8 || h * 10 > sh * 8 {
        return false;
    }

    let style = window_long(hwnd, GWL_STYLE) as u32;

    // タイトルバーも表題も持たないものは除外する。
    //
    // Chromium 系アプリのポップアップ (オートフィル候補など) やツールチップは
    // 小さくリサイズ不可なため、これがないと「ダイアログらしい」と判定されて
    // 大量に走査されてしまう。
    //
    // ただし WS_CAPTION だけを条件にすると、タイトルバーを自前で描くアプリ
    // (fre:ac の smooth ツールキットなど) のダイアログを取りこぼす。
    // 表題テキストを持っていれば、装飾が独自でも「名前のあるウィンドウ」として扱う。
    let has_caption = style & WS_CAPTION == WS_CAPTION;
    if !has_caption && window_text(hwnd).trim().is_empty() {
        return false;
    }

    let has_owner = !owner_of(hwnd).is_null();
    let resizable = style & WS_THICKFRAME != 0;
    // 画面の半分未満に収まる小さなウィンドウ
    let small = w * 2 <= sw && h * 2 <= sh;

    // オーナー持ち（＝誰かが出したダイアログ）か、リサイズできないか、
    // 十分に小さいウィンドウであること。
    //
    // エクスプローラのファイル操作ダイアログ (OperationStatusWindow) は
    // オーナーを持たずリサイズも可能なので、サイズによる救済が必要になる。
    has_owner || !resizable || small
}

/// 既定で「OK 相当」とみなすボタンラベル。
const DEFAULT_LABELS: &[&str] = &["OK", "はい", "Yes", "了解", "続行", "Continue"];

/// 32bit / 64bit の両方で GetWindowLong(Ptr)W を使えるようにする薄いラッパ。
#[cfg(target_pointer_width = "64")]
unsafe fn window_long(hwnd: HWND, index: i32) -> isize {
    windows_sys::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW(hwnd, index)
}

#[cfg(not(target_pointer_width = "64"))]
unsafe fn window_long(hwnd: HWND, index: i32) -> isize {
    windows_sys::Win32::UI::WindowsAndMessaging::GetWindowLongW(hwnd, index) as isize
}

pub unsafe fn class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    let n = GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
    if n <= 0 {
        String::new()
    } else {
        from_wide(&buf[..n as usize])
    }
}

/// ウィンドウクラス名を、`String` を確保せずに比較する（ASCII 大文字小文字は無視）。
///
/// クラス名の照合はイベントごとに走るため、`class_name()` で毎回
/// `String` を確保するのは無駄が大きい。
pub unsafe fn class_is(hwnd: HWND, expect: &str) -> bool {
    let mut buf = [0u16; 128];
    let n = GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
    if n <= 0 {
        return false;
    }
    let actual = &buf[..n as usize];
    let mut want = expect.encode_utf16();
    for &a in actual {
        match want.next() {
            Some(b) => {
                let (a, b) = (to_ascii_lower(a), to_ascii_lower(b));
                if a != b {
                    return false;
                }
            }
            None => return false,
        }
    }
    want.next().is_none()
}

/// ウィンドウクラス名に指定した文字列が含まれるか（ASCII 大文字小文字は無視）。
///
/// ボタンのクラス名は標準の `Button` とは限らない。Inno Setup は `TNewButton`、
/// Delphi/VCL は `TButton`、旧 VB は `ThunderRT6CommandButton` のように、
/// 独自クラスでも標準ボタンの挙動を継承していることが多い。
pub unsafe fn class_contains(hwnd: HWND, needle: &str) -> bool {
    let mut buf = [0u16; 128];
    let n = GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
    if n <= 0 {
        return false;
    }
    let actual: Vec<u16> = buf[..n as usize]
        .iter()
        .map(|&c| to_ascii_lower(c))
        .collect();
    let want: Vec<u16> = needle.encode_utf16().map(to_ascii_lower).collect();
    if want.is_empty() || want.len() > actual.len() {
        return false;
    }
    actual.windows(want.len()).any(|w| w == want.as_slice())
}

fn to_ascii_lower(c: u16) -> u16 {
    if (b'A' as u16..=b'Z' as u16).contains(&c) {
        c + 32
    } else {
        c
    }
}

/// ブラウザ系ウィンドウのクラス名。
///
/// Web ページは任意の UI を作れるため、「既定ボタンらしい配置」だけを根拠に
/// カーソルを移動すると、ブロック・削除・購入といった重い操作のボタンに
/// 当たってしまう。アプリのダイアログとは区別して扱う。
const BROWSER_CLASSES: &[&str] = &[
    "Chrome_WidgetWin_1", // Chrome / Edge / Electron
    "MozillaWindowClass", // Firefox
    "IEFrame",            // Internet Explorer
];

/// ブラウザのウィンドウか。
pub unsafe fn is_browser_window(hwnd: HWND) -> bool {
    BROWSER_CLASSES.iter().any(|c| class_is(hwnd, c))
}

pub unsafe fn window_text(hwnd: HWND) -> String {
    let mut buf = [0u16; 512];
    let n = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
    if n <= 0 {
        String::new()
    } else {
        from_wide(&buf[..n as usize])
    }
}

/// 子ウィンドウでない（トップレベルの）ウィンドウか。
pub unsafe fn is_top_level(hwnd: HWND) -> bool {
    if hwnd.is_null() || IsWindow(hwnd) == 0 {
        return false;
    }
    window_long(hwnd, GWL_STYLE) as u32 & WS_CHILD == 0
}

/// このウィンドウを「ダイアログ」として扱うか判定する。
pub unsafe fn is_candidate_dialog(hwnd: HWND, standard_only: bool) -> bool {
    if hwnd.is_null() || IsWindow(hwnd) == 0 || IsWindowVisible(hwnd) == 0 {
        return false;
    }
    let style = window_long(hwnd, GWL_STYLE) as u32;
    // 子ウィンドウはトップレベルのダイアログではない
    if style & WS_CHILD != 0 {
        return false;
    }
    if class_is(hwnd, "#32770") {
        return true;
    }
    if standard_only {
        return false;
    }
    // 緩いモード: タイトルのあるポップアップも対象にする
    style & WS_POPUP != 0 && !window_text(hwnd).is_empty()
}

unsafe extern "system" fn collect_children(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let list = &mut *(lparam as *mut Vec<isize>);
    list.push(hwnd as isize);
    if list.len() > 512 {
        return 0; // 異常に子が多いウィンドウでは打ち切る
    }
    1
}

unsafe fn is_usable_button(hwnd: HWND) -> bool {
    if hwnd.is_null()
        || IsWindow(hwnd) == 0
        || IsWindowVisible(hwnd) == 0
        || IsWindowEnabled(hwnd) == 0
    {
        return false;
    }
    // 標準の "Button" 以外に TNewButton / TButton などの派生クラスも受け入れる
    if !class_contains(hwnd, "button") {
        return false;
    }
    let bs = window_long(hwnd, GWL_STYLE) as u32 & BS_TYPEMASK;
    // チェックボックスやラジオボタンを除外し、押しボタンだけを許可する。
    // オーナードローも見た目が違うだけの押しボタンなので受け入れる
    bs == BS_PUSHBUTTON || bs == BS_DEFPUSHBUTTON || bs == BS_OWNERDRAW
}

/// "&はい(&Y)" のようなラベルからニーモニックを取り除いて比較用に正規化する。
pub fn normalize_label(s: &str) -> String {
    let t = s.replace('&', "");
    let t = t.trim().to_string();
    strip_mnemonic_paren(&t).trim().to_string()
}

/// 「取り消し」に相当するボタンのラベル。
///
/// ダイアログのボタン行にはほぼ必ずこれらが並ぶため、
/// 「いまダイアログが開いている」ことの判定材料として使う。
const CANCEL_LABELS: &[&str] = &[
    "キャンセル",
    "Cancel",
    "いいえ",
    "No",
    "中止",
    "Abort",
    "後で",
    "Not now",
    "Later",
];

/// ラベルが「取り消し」相当か。
pub fn is_cancel_label(raw: &str) -> bool {
    let label = normalize_label(raw);
    !label.is_empty() && CANCEL_LABELS.iter().any(|c| c.eq_ignore_ascii_case(&label))
}

/// ラベルが OK 相当ならスコアを返す（0 = 該当なし）。HWND 版と UIA 版で共用する。
pub fn label_score(raw: &str, extra: &[String]) -> i32 {
    let label = normalize_label(raw);
    if label.is_empty() {
        return 0;
    }
    if DEFAULT_LABELS
        .iter()
        .any(|l| l.eq_ignore_ascii_case(&label))
    {
        return 50;
    }
    if extra
        .iter()
        .any(|l| normalize_label(l).eq_ignore_ascii_case(&label))
    {
        return 40;
    }
    0
}

fn strip_mnemonic_paren(s: &str) -> &str {
    for (open, close) in [('(', ')'), ('（', '）')] {
        if s.ends_with(close) {
            if let Some(i) = s.rfind(open) {
                let inner = &s[i + open.len_utf8()..s.len() - close.len_utf8()];
                let mut chars = inner.chars();
                if let (Some(c), None) = (chars.next(), chars.next()) {
                    if c.is_ascii_alphanumeric() {
                        return &s[..i];
                    }
                }
            }
        }
    }
    s
}

/// ダイアログ直下（再帰的）の子ウィンドウを列挙する。
pub unsafe fn child_windows(dlg: HWND) -> Vec<isize> {
    let mut list: Vec<isize> = Vec::new();
    EnumChildWindows(
        dlg,
        Some(collect_children),
        &mut list as *mut Vec<isize> as LPARAM,
    );
    list
}

/// 診断用: 子コントロールの一覧を 1 行の文字列にまとめる。
pub unsafe fn dump_children(dlg: HWND) -> String {
    let mut out = Vec::new();
    for raw in child_windows(dlg) {
        let hwnd = raw as HWND;
        let cls = class_name(hwnd);
        let text = window_text(hwnd);
        let id = window_long(hwnd, GWL_ID) as i32;
        let style = window_long(hwnd, GWL_STYLE) as u32;
        out.push(format!(
            "{{cls={} id={} text=\"{}\" bs={:#x} vis={} en={}}}",
            cls,
            id,
            text,
            style & BS_TYPEMASK,
            IsWindowVisible(hwnd),
            IsWindowEnabled(hwnd)
        ));
    }
    if out.is_empty() {
        "(子ウィンドウなし)".to_string()
    } else {
        out.join(" ")
    }
}

/// ファイル/フォルダー選択ダイアログに特有の子ウィンドウクラス。
///
/// 「開く」「名前を付けて保存」「フォルダーの選択」はいずれもクラスが `#32770` で、
/// 決定ボタンのコントロール ID も `IDOK` なので、ラベルやクラスだけでは
/// 通常のダイアログと区別できない。ファイル一覧ビューを内包しているかで判定する。
///
/// `DirectUIHWND` や `DUIViewWndClassName` はシェルのダイアログ全般で使われており、
/// 「ファイルの置換またはスキップ」なども該当してしまうため目印には使えない。
/// ファイル一覧そのものである `SHELLDLL_DefView` だけを見る。
const FILE_DIALOG_MARKERS: &[&str] = &["SHELLDLL_DefView"];
/// プロパティシートが持つタブコントロール
const TAB_CONTROL_CLASS: &str = "SysTabControl32";

/// コモンダイアログ（ファイル/フォルダー選択）かどうか。
///
/// 目印はファイル一覧ビュー (`SHELLDLL_DefView`) だが、これだけでは足りない。
/// ファイルのプロパティも「以前のバージョン」タブなどでシェルのビューを作るため、
/// そのタブを開いた後だけ選択ダイアログと誤判定されてしまう。
///
/// 決定ボタンの ID も区別に使えない。プロパティの「OK」も `IDOK` だからである。
///
/// 実際に効くのはタブの有無で、ファイル選択ダイアログはタブを持たず、
/// プロパティシートは必ず持つ。表示中のビューであることと併せて判定する。
pub unsafe fn is_file_dialog(hwnd: HWND) -> bool {
    let children = child_windows(hwnd);

    let has_visible_view = children.iter().any(|&raw| {
        let child = raw as HWND;
        IsWindowVisible(child) != 0 && FILE_DIALOG_MARKERS.iter().any(|m| class_is(child, m))
    });
    if !has_visible_view {
        return false;
    }

    // タブがあればプロパティシート。選択ダイアログではない
    !children
        .iter()
        .any(|&raw| class_is(raw as HWND, TAB_CONTROL_CLASS))
}

/// 進捗表示（プログレスバー）を持つウィンドウか。
///
/// 「コピー中」「展開中」のような処理中ダイアログを除外するために使う。
pub unsafe fn has_progress_bar(hwnd: HWND) -> bool {
    for raw in child_windows(hwnd) {
        let h = raw as HWND;
        if IsWindowVisible(h) == 0 {
            continue;
        }
        if class_is(h, "msctls_progress32") {
            return true;
        }
    }
    false
}

/// ウィンドウを所有するプロセスの実行ファイル名（例: `peazip.exe`）。
pub unsafe fn process_name(hwnd: HWND) -> Option<String> {
    let mut pid: u32 = 0;
    GetWindowThreadProcessId(hwnd, &mut pid);
    if pid == 0 {
        return None;
    }

    let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
    if handle.is_null() {
        return None;
    }

    let mut buf = [0u16; 520];
    let mut len = buf.len() as u32;
    let ok = QueryFullProcessImageNameW(handle, 0, buf.as_mut_ptr(), &mut len);
    CloseHandle(handle);
    if ok == 0 {
        return None;
    }

    let full = from_wide(&buf[..len as usize]);
    // パスを落として実行ファイル名だけにする
    let name = full
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(&full)
        .to_ascii_lowercase();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

/// ラベルが一致しなくても、ダイアログの既定ボタンなら拾う。
///
/// 次のどちらかを満たす `BS_DEFPUSHBUTTON` を既定ボタンとみなす。
///
/// 1. 同じ行に「キャンセル」相当のボタンが並んでいる
///    （「次へ / キャンセル」のようなウィザードのボタン行）
/// 2. ダイアログ下部のボタン行に置かれている
///    （「閉じる」だけが右下にあるような情報ダイアログ）
///
/// この関数はダイアログと判定済みのウィンドウにしか呼ばれないため、
/// 通常のアプリウィンドウの既定ボタンを掴む心配はない。
pub unsafe fn find_default_button(dlg: HWND) -> Option<HWND> {
    let dialog_rect = window_rect(dlg)?;
    // 下端から 25% を「ボタン行」とみなす
    let footer_top = dialog_rect.bottom - (dialog_rect.bottom - dialog_rect.top) / 4;

    let mut default_button: Option<(HWND, RECT)> = None;
    let mut cancels: Vec<RECT> = Vec::new();

    for raw in child_windows(dlg) {
        let hwnd = raw as HWND;
        if !is_usable_button(hwnd) {
            continue;
        }
        let text = window_text(hwnd);
        if text.trim().is_empty() {
            continue;
        }
        let Some(rect) = window_rect(hwnd) else {
            continue;
        };

        let cancel = is_cancel_label(&text);

        // 「キャンセル」相当は、既定ボタンであっても移動先にしない。
        //
        // インストーラの実行中など、Next が無効化されて Cancel が既定ボタンに
        // なることがある。そこへカーソルを運ぶのは意図と正反対になる。
        let style = window_long(hwnd, GWL_STYLE) as u32;
        if !cancel && style & BS_TYPEMASK == BS_DEFPUSHBUTTON && default_button.is_none() {
            default_button = Some((hwnd, rect));
        }
        if cancel {
            cancels.push(rect);
        }
    }

    let (hwnd, rect) = default_button?;

    let scale = dpi_scale_percent(dlg);
    let has_cancel_beside = cancels.iter().any(|c| same_button_row(c, &rect, scale));
    let in_footer = (rect.top + rect.bottom) / 2 >= footer_top;

    if has_cancel_beside || in_footer {
        Some(hwnd)
    } else {
        None
    }
}

/// ウィンドウが最小化されているか。
///
/// 最小化してもウィンドウは `WS_VISIBLE` のままなので、`IsWindowVisible` では
/// 見分けられない。矩形は (-32000, -32000) 付近になるため、そのまま
/// ボタンの中心を計算すると画面外の座標が出てくる。
pub unsafe fn is_minimized(hwnd: HWND) -> bool {
    IsIconic(hwnd) != 0
}

/// その座標がいずれかのモニタ上にあるか。
///
/// カーソルを送る直前の最後の関門。ここを通さないと、最小化された
/// ウィンドウやまだレイアウトが確定していないウィンドウから
/// 拾った座標で、カーソルが画面の隅に張り付く。
pub unsafe fn point_is_on_a_monitor(p: &POINT) -> bool {
    !MonitorFromPoint(*p, MONITOR_DEFAULTTONULL).is_null()
}

/// ウィンドウの拡大率をパーセントで返す（96 dpi = 100）。
///
/// プロセスはモニタ単位の DPI 対応なので、`GetWindowRect` が返す座標は
/// 物理ピクセルになる。ピクセル数で決め打ちしたしきい値をそのまま使うと、
/// 拡大表示の環境では実質的な距離が縮み、同じダイアログでも判定結果が
/// 変わってしまうため、これで補正する。
///
/// 取得できない場合は 100（等倍）。下限も 100 にして、従来より
/// 判定が厳しくなる方向へは動かさない。
pub unsafe fn dpi_scale_percent(hwnd: HWND) -> i32 {
    let dpi = GetDpiForWindow(hwnd);
    if dpi == 0 {
        return 100;
    }
    (dpi as i32 * 100 / 96).clamp(100, 400)
}

/// ダイアログ内の OK ボタンを探す。
///
/// ラベルの一致を必須条件にする。以前は `#32770` に限って
/// `GetDlgItem(dlg, IDOK)` をラベル照合なしで採用する近道があったが、これを外した。
///
/// Win32 アプリはメインウィンドウをダイアログとして作ることがある。その場合
/// クラスは `#32770` で、主要な操作ボタンが ID=1 を持つ。近道があると、
/// たとえばファイル転送ツールの「実行」ボタン——押せばコピーや削除が始まる——を
/// 「OK ボタン」として掴んでしまう。実際にそうなっていた。
///
/// 近道が担っていた「MessageBox の OK を確実に選ぶ」役割は、下のスコアリングが
/// `id == IDOK` に +100 を与えることで既に果たされている。ラベル "OK" と
/// 合わせて 160 点になり、他の候補に負けることはない。
pub unsafe fn find_ok_button(dlg: HWND, extra: &[String]) -> Option<HWND> {
    // 子ウィンドウを走査してラベルとスタイルからスコアリングする
    let list = child_windows(dlg);

    let mut best: Option<(i32, HWND)> = None;

    for raw in list {
        let hwnd = raw as HWND;
        if !is_usable_button(hwnd) {
            continue;
        }

        // ラベルの一致を必須条件にする。
        // コントロール ID や既定ボタンかどうかは、候補が複数あるときの
        // 優先順位付けにのみ使う。ID だけを根拠に採用すると、
        // たまたま ID=1 を持つ無関係なコントロールを掴んでしまう。
        let base = label_score(&window_text(hwnd), extra);
        if base == 0 {
            continue;
        }

        let id = window_long(hwnd, GWL_ID) as i32;
        let style = window_long(hwnd, GWL_STYLE) as u32;
        let mut score = base;
        if id == IDOK {
            score += 100;
        }
        if style & BS_TYPEMASK == BS_DEFPUSHBUTTON {
            score += 10;
        }
        if best.is_none_or(|(s, _)| score > s) {
            best = Some((score, hwnd));
        }
    }

    best.map(|(_, h)| h)
}

pub unsafe fn window_rect(hwnd: HWND) -> Option<RECT> {
    let mut r: RECT = std::mem::zeroed();
    if GetWindowRect(hwnd, &mut r) == 0 {
        return None;
    }
    if r.right <= r.left || r.bottom <= r.top {
        return None;
    }
    Some(r)
}

pub fn center_of(r: &RECT) -> POINT {
    POINT {
        x: r.left + (r.right - r.left) / 2,
        y: r.top + (r.bottom - r.top) / 2,
    }
}

/// 2 つのボタンが同じ「まとまり」に並んでいるか。
///
/// ダイアログのボタン群を見分けるために使う。並びには 2 通りある。
///
/// * 横並び — 「OK / キャンセル」のような一般的なボタン行
/// * 縦並び — TaskDialog のコマンドリンクのように選択肢が積まれている形
///
/// どちらか一方の軸で重なりがあり、もう一方の間隔が近ければ同じまとまりとみなす。
///
/// `scale` はウィンドウの拡大率（パーセント、等倍で 100）。座標は物理ピクセルで
/// 来るため、しきい値のほうを拡大率に合わせないと、拡大表示の環境で
/// 同じレイアウトのダイアログが別物として扱われる。
pub fn same_button_row(a: &RECT, b: &RECT, scale: i32) -> bool {
    let overlaps_vertically = a.top < b.bottom && b.top < a.bottom;
    let overlaps_horizontally = a.left < b.right && b.left < a.right;

    let horizontal_gap = if b.left >= a.right {
        b.left - a.right
    } else if a.left >= b.right {
        a.left - b.right
    } else {
        0
    };
    let vertical_gap = if b.top >= a.bottom {
        b.top - a.bottom
    } else if a.top >= b.bottom {
        a.top - b.bottom
    } else {
        0
    };

    // 等倍のときの許容間隔。縦並びは狭めに見る（離れた位置のボタンまで拾わないため）
    let scale = scale.max(1);
    let horizontal_limit = 600 * scale / 100;
    let vertical_limit = 120 * scale / 100;

    (overlaps_vertically && horizontal_gap <= horizontal_limit)
        || (overlaps_horizontally && vertical_gap <= vertical_limit)
}

pub fn point_in_rect(p: &POINT, r: &RECT) -> bool {
    p.x >= r.left && p.x < r.right && p.y >= r.top && p.y < r.bottom
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
        RECT {
            left,
            top,
            right,
            bottom,
        }
    }

    #[test]
    fn mnemonics_are_stripped_from_labels() {
        assert_eq!(normalize_label("&OK"), "OK");
        assert_eq!(normalize_label("保存(&S)"), "保存");
        assert_eq!(
            normalize_label("ファイルを置き換える(R)"),
            "ファイルを置き換える"
        );
        assert_eq!(normalize_label("次へ (&N)"), "次へ");
        // 全角の括弧も扱う
        assert_eq!(normalize_label("はい（Y）"), "はい");
        // 括弧の中身が 1 文字の英数字でなければ残す
        assert_eq!(normalize_label("設定 (詳細)"), "設定 (詳細)");
    }

    #[test]
    fn default_labels_are_recognised() {
        let none: &[String] = &[];
        assert!(label_score("OK", none) > 0);
        assert!(label_score("はい", none) > 0);
        assert!(label_score("&Yes", none) > 0);
        assert!(label_score("Continue", none) > 0);
        assert_eq!(label_score("Delete", none), 0);
        assert_eq!(label_score("", none), 0);
    }

    #[test]
    fn extra_labels_are_honoured() {
        let extra = vec!["ファイルを置き換える".to_string()];
        assert!(label_score("ファイルを置き換える(R)", &extra) > 0);
        assert_eq!(label_score("スキップする", &extra), 0);
    }

    #[test]
    fn cancel_labels_are_recognised() {
        assert!(is_cancel_label("キャンセル"));
        assert!(is_cancel_label("Cancel"));
        assert!(is_cancel_label("&Cancel"));
        assert!(is_cancel_label("Not now"));
        assert!(is_cancel_label("いいえ(&N)"));
        assert!(!is_cancel_label("OK"));
        assert!(!is_cancel_label(""));
    }

    /// OK と Cancel が横に並んだ、一般的なボタン行。
    #[test]
    fn horizontal_button_row_is_detected() {
        let ok = rect(100, 500, 180, 530);
        let cancel = rect(190, 500, 270, 530);
        assert!(same_button_row(&ok, &cancel, 100));
    }

    /// TaskDialog のコマンドリンクのように縦に積まれた並び。
    #[test]
    fn vertical_button_stack_is_detected() {
        let update = rect(100, 300, 400, 340);
        let not_now = rect(100, 360, 400, 400);
        assert!(same_button_row(&update, &not_now, 100));
    }

    /// 離れた位置のボタンは同じ並びとみなさない。
    #[test]
    fn distant_buttons_are_not_a_row() {
        let a = rect(100, 500, 180, 530);
        // 横に 700px 離れている
        let far_right = rect(880, 500, 960, 530);
        assert!(!same_button_row(&a, &far_right, 100));
        // 縦に 200px 離れている
        let far_below = rect(100, 730, 180, 760);
        assert!(!same_button_row(&a, &far_below, 100));
    }

    /// 拡大表示では、同じレイアウトが物理ピクセルで 2 倍の間隔になる。
    ///
    /// しきい値を補正しないと、200% の環境でだけ既定ボタンへの追従が
    /// 効かなくなる。作者の環境が等倍だと気づけない種類の不具合なので、
    /// テストで押さえておく。
    #[test]
    fn thresholds_follow_the_display_scale() {
        // 等倍で 400px 離れたボタン対（同じ行と判定される）
        let a = rect(100, 500, 180, 530);
        let b = rect(580, 500, 660, 530);
        assert!(same_button_row(&a, &b, 100));

        // 200% では同じレイアウトが 800px 離れて見える
        let a2 = rect(200, 1000, 360, 1060);
        let b2 = rect(1160, 1000, 1320, 1060);
        assert!(
            !same_button_row(&a2, &b2, 100),
            "補正なしでは取りこぼす（この不具合の再現）"
        );
        assert!(
            same_button_row(&a2, &b2, 200),
            "拡大率を渡せば同じ行と判定される"
        );
    }
}
