//! UI Automation によるボタン検索。
//!
//! windows-sys は COM インターフェースをほとんど提供しないため、必要な分だけ
//! vtable を手書きで定義している。使わないスロットは `usize` のプレースホルダに
//! しておき、絶対に呼ばないことでシグネチャの誤りによる事故を避ける。

use std::cell::Cell;
use std::ffi::c_void;
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{HWND, RECT};

use crate::dialog::label_score;
use crate::log;

type Hr = i32;
const S_OK: Hr = 0;

const CLSCTX_INPROC_SERVER: u32 = 0x1;
const COINIT_APARTMENTTHREADED: u32 = 0x2;

/// TreeScope_Descendants
const TREESCOPE_DESCENDANTS: i32 = 4;
/// UIA_ButtonControlTypeId
const CONTROL_TYPE_BUTTON: i32 = 50000;
/// UIA_HyperlinkControlTypeId。TaskDialog のコマンドリンクがこの型になることがある
const CONTROL_TYPE_HYPERLINK: i32 = 50005;
/// UIA_ProgressBarControlTypeId。処理中ダイアログの判別に使う
const CONTROL_TYPE_PROGRESSBAR: i32 = 50012;

#[repr(C)]
#[derive(Clone, Copy)]
struct Guid {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

/// CLSID_CUIAutomation {FF48DBA4-60EF-4201-AA87-54103EEF594E}
const CLSID_CUIAUTOMATION: Guid = Guid {
    data1: 0xFF48DBA4,
    data2: 0x60EF,
    data3: 0x4201,
    data4: [0xAA, 0x87, 0x54, 0x10, 0x3E, 0xEF, 0x59, 0x4E],
};

/// IID_IUIAutomation {30CBE57D-D9D0-452A-AB13-7AC5AC4825EE}
const IID_IUIAUTOMATION: Guid = Guid {
    data1: 0x30CBE57D,
    data2: 0xD9D0,
    data3: 0x452A,
    data4: [0xAB, 0x13, 0x7A, 0xC5, 0xAC, 0x48, 0x25, 0xEE],
};

#[link(name = "ole32")]
extern "system" {
    fn CoInitializeEx(reserved: *mut c_void, co_init: u32) -> Hr;
    fn CoUninitialize();
    fn CoCreateInstance(
        rclsid: *const Guid,
        unk_outer: *mut c_void,
        cls_context: u32,
        riid: *const Guid,
        ppv: *mut *mut c_void,
    ) -> Hr;
}

#[link(name = "oleaut32")]
extern "system" {
    fn SysFreeString(bstr: *mut u16);
}

// ---- vtable 定義 --------------------------------------------------------

#[repr(C)]
#[allow(dead_code)]
struct UnknownVtbl {
    query_interface: usize,
    add_ref: usize,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
}

#[repr(C)]
#[allow(dead_code)]
struct AutomationVtbl {
    query_interface: usize,
    add_ref: usize,
    release: usize,
    compare_elements: usize,
    compare_runtime_ids: usize,
    get_root_element: usize,
    element_from_handle:
        unsafe extern "system" fn(*mut c_void, *mut c_void, *mut *mut c_void) -> Hr,
    element_from_point: usize,
    get_focused_element: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> Hr,
    get_root_element_build_cache: usize,
    element_from_handle_build_cache: usize,
    element_from_point_build_cache: usize,
    get_focused_element_build_cache: usize,
    create_tree_walker: usize,
    get_control_view_walker: usize,
    get_content_view_walker: usize,
    get_raw_view_walker: usize,
    get_raw_view_condition: usize,
    get_control_view_condition: usize,
    get_content_view_condition: usize,
    create_cache_request: usize,
    create_true_condition: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> Hr,
    create_false_condition: usize,
    create_property_condition:
        unsafe extern "system" fn(*mut c_void, i32, Variant, *mut *mut c_void) -> Hr,
    create_property_condition_ex: usize,
    create_and_condition: usize,
    create_and_condition_from_array: usize,
    create_and_condition_from_native_array: usize,
    create_or_condition:
        unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, *mut *mut c_void) -> Hr,
}

/// VARIANT の共用体部分の大きさを揃えるための詰め物。
///
/// VARIANT は 8 バイトのヘッダに共用体が続く形で、共用体にはポインタ 2 つ分の
/// `BRECORD` が含まれる。したがって全体は x64 で 24 バイト、x86 で 16 バイトになる。
#[cfg(target_pointer_width = "64")]
type VariantPad = [u8; 8];
#[cfg(not(target_pointer_width = "64"))]
type VariantPad = [u8; 0];

/// COM の VARIANT。
/// `CreatePropertyCondition` は値渡しで受け取るのでレイアウトを厳密に合わせる。
#[repr(C)]
#[derive(Clone, Copy)]
struct Variant {
    vt: u16,
    reserved1: u16,
    reserved2: u16,
    reserved3: u16,
    value: i64,
    padding: VariantPad,
}

/// VT_I4 の VARIANT を作る。
fn variant_i4(v: i32) -> Variant {
    Variant {
        vt: 3, // VT_I4
        reserved1: 0,
        reserved2: 0,
        reserved3: 0,
        value: v as i64,
        padding: VariantPad::default(),
    }
}

#[repr(C)]
#[allow(dead_code)]
struct ElementVtbl {
    query_interface: usize,
    add_ref: usize,
    release: usize,
    set_focus: usize,
    get_runtime_id: usize,
    find_first: usize,
    find_all: unsafe extern "system" fn(*mut c_void, i32, *mut c_void, *mut *mut c_void) -> Hr,
    find_first_build_cache: usize,
    find_all_build_cache: usize,
    build_updated_cache: usize,
    get_current_property_value: usize,
    get_current_property_value_ex: usize,
    get_cached_property_value: usize,
    get_cached_property_value_ex: usize,
    get_current_pattern_as: usize,
    get_cached_pattern_as: usize,
    get_current_pattern: usize,
    get_cached_pattern: usize,
    get_cached_parent: usize,
    get_cached_children: usize,
    get_current_process_id: usize,
    get_current_control_type: unsafe extern "system" fn(*mut c_void, *mut i32) -> Hr,
    get_current_localized_control_type: usize,
    get_current_name: unsafe extern "system" fn(*mut c_void, *mut *mut u16) -> Hr,
    get_current_accelerator_key: usize,
    get_current_access_key: usize,
    get_current_has_keyboard_focus: usize,
    get_current_is_keyboard_focusable: usize,
    get_current_is_enabled: unsafe extern "system" fn(*mut c_void, *mut i32) -> Hr,
    get_current_automation_id: usize,
    get_current_class_name: usize,
    get_current_help_text: usize,
    get_current_culture: usize,
    get_current_is_control_element: usize,
    get_current_is_content_element: usize,
    get_current_is_password: usize,
    get_current_native_window_handle: usize,
    get_current_item_type: usize,
    get_current_is_offscreen: unsafe extern "system" fn(*mut c_void, *mut i32) -> Hr,
    get_current_orientation: usize,
    get_current_framework_id: usize,
    get_current_is_required_for_form: usize,
    get_current_item_status: usize,
    get_current_bounding_rectangle: unsafe extern "system" fn(*mut c_void, *mut RECT) -> Hr,
}

#[repr(C)]
#[allow(dead_code)]
struct ElementArrayVtbl {
    query_interface: usize,
    add_ref: usize,
    release: usize,
    get_length: unsafe extern "system" fn(*mut c_void, *mut i32) -> Hr,
    get_element: unsafe extern "system" fn(*mut c_void, i32, *mut *mut c_void) -> Hr,
}

/// COM オブジェクトポインタから vtable を取り出す。
unsafe fn vt<V>(p: *mut c_void) -> *const V {
    *(p as *const *const V)
}

unsafe fn release(p: *mut c_void) {
    if !p.is_null() {
        ((*vt::<UnknownVtbl>(p)).release)(p);
    }
}

unsafe fn bstr_to_string(b: *mut u16) -> String {
    if b.is_null() {
        return String::new();
    }
    let mut len = 0usize;
    // 上限の判定を先に行う。順序を逆にすると、上限に達した時点でも
    // 一度余計に参照してしまう。
    while len < 4096 && *b.add(len) != 0 {
        len += 1;
    }
    let s = String::from_utf16_lossy(std::slice::from_raw_parts(b, len));
    SysFreeString(b);
    s
}

// ---- インスタンス管理 ---------------------------------------------------

thread_local! {
    /// IUIAutomation のインスタンス（0 = 未取得 / usize::MAX = 取得失敗）
    static AUTOMATION: Cell<usize> = const { Cell::new(0) };
    static COM_READY: Cell<bool> = const { Cell::new(false) };
}

/// スレッドで COM を初期化する。メッセージループを持つスレッドなので STA。
pub unsafe fn init_com() {
    if COM_READY.with(|c| c.get()) {
        return;
    }
    let hr = CoInitializeEx(null_mut(), COINIT_APARTMENTTHREADED);
    // S_FALSE (1) は「既に初期化済み」なので成功扱い
    if hr >= 0 {
        COM_READY.with(|c| c.set(true));
    } else {
        log::info(&format!("CoInitializeEx に失敗しました hr={:#x}", hr));
    }
}

pub unsafe fn uninit_com() {
    let p = AUTOMATION.with(|c| c.replace(0));
    if p != 0 && p != usize::MAX {
        release(p as *mut c_void);
    }
    if COM_READY.with(|c| c.replace(false)) {
        CoUninitialize();
    }
}

unsafe fn automation() -> *mut c_void {
    let cached = AUTOMATION.with(|c| c.get());
    if cached == usize::MAX {
        return null_mut();
    }
    if cached != 0 {
        return cached as *mut c_void;
    }

    init_com();
    let mut p: *mut c_void = null_mut();
    let hr = CoCreateInstance(
        &CLSID_CUIAUTOMATION,
        null_mut(),
        CLSCTX_INPROC_SERVER,
        &IID_IUIAUTOMATION,
        &mut p,
    );
    if hr != S_OK || p.is_null() {
        log::info(&format!(
            "UI Automation の初期化に失敗しました hr={:#x}",
            hr
        ));
        AUTOMATION.with(|c| c.set(usize::MAX));
        return null_mut();
    }
    AUTOMATION.with(|c| c.set(p as usize));
    p
}

// ---- 探索 ---------------------------------------------------------------

/// UIA_ControlTypePropertyId
const UIA_CONTROL_TYPE_PROPERTY_ID: i32 = 30003;

/// 指定した ControlType にマッチする条件を作る。
unsafe fn control_type_condition(auto: *mut c_void, control_type: i32) -> *mut c_void {
    let mut cond: *mut c_void = null_mut();
    let hr = ((*vt::<AutomationVtbl>(auto)).create_property_condition)(
        auto,
        UIA_CONTROL_TYPE_PROPERTY_ID,
        variant_i4(control_type),
        &mut cond,
    );
    if hr != S_OK {
        return null_mut();
    }
    cond
}

/// 探索対象の ControlType だけに絞る条件を組み立てる。
///
/// `CreateTrueCondition` で全要素を取ると、ブラウザのようなウィンドウでは
/// 数千要素が返り、そのすべてに対してクロスプロセスのプロパティ取得が走る。
/// UIA 側で絞らせることで、返る要素数を 1〜2 桁減らせる。
///
/// 条件を組めなかった場合は `CreateTrueCondition` にフォールバックする。
unsafe fn build_condition(auto: *mut c_void) -> *mut c_void {
    let vtbl = vt::<AutomationVtbl>(auto);

    let button = control_type_condition(auto, CONTROL_TYPE_BUTTON);
    let hyperlink = control_type_condition(auto, CONTROL_TYPE_HYPERLINK);
    let progress = control_type_condition(auto, CONTROL_TYPE_PROGRESSBAR);

    let mut combined: *mut c_void = null_mut();
    if !button.is_null() && !hyperlink.is_null() && !progress.is_null() {
        let mut or1: *mut c_void = null_mut();
        if ((*vtbl).create_or_condition)(auto, button, hyperlink, &mut or1) == S_OK
            && !or1.is_null()
        {
            let mut or2: *mut c_void = null_mut();
            if ((*vtbl).create_or_condition)(auto, or1, progress, &mut or2) == S_OK {
                combined = or2;
            }
            release(or1);
        }
    }

    // OR 条件側が参照を保持するので、個々の条件はここで解放してよい
    release(button);
    release(hyperlink);
    release(progress);

    if !combined.is_null() {
        return combined;
    }

    // フォールバック: 全要素を対象にする
    let mut all: *mut c_void = null_mut();
    if ((*vtbl).create_true_condition)(auto, &mut all) != S_OK {
        return null_mut();
    }
    all
}

pub struct Found {
    pub rect: RECT,
    pub name: String,
}

/// 走査で拾った押下可能な要素。
pub struct ButtonInfo {
    pub name: String,
    pub rect: RECT,
}

/// 走査中に拾った付随情報（診断と除外判定に使う）。
#[derive(Default)]
pub struct ScanInfo {
    /// 見つかった押下可能な要素の名前
    pub seen: Vec<String>,
    /// 表示中のプログレスバーがあったか
    pub progress: bool,
}

/// ウィンドウ配下の押下可能な要素をすべて集める。
///
/// `info.seen` には見つかった押下可能な要素の名前が入る（診断ログ用）。
///
/// `ControlType` は UIA 側で Button / Hyperlink / ProgressBar に絞られている。
unsafe fn collect_buttons(hwnd: HWND, max_elements: usize, info: &mut ScanInfo) -> Vec<ButtonInfo> {
    let mut out = Vec::new();
    let auto = automation();
    if auto.is_null() {
        return out;
    }

    let mut root: *mut c_void = null_mut();
    if ((*vt::<AutomationVtbl>(auto)).element_from_handle)(auto, hwnd, &mut root) != S_OK
        || root.is_null()
    {
        return out;
    }

    // ControlType で UIA 側に絞り込ませる
    let cond = build_condition(auto);
    if cond.is_null() {
        release(root);
        return out;
    }

    let mut array: *mut c_void = null_mut();
    let hr = ((*vt::<ElementVtbl>(root)).find_all)(root, TREESCOPE_DESCENDANTS, cond, &mut array);
    release(cond);
    release(root);

    if hr != S_OK || array.is_null() {
        return out;
    }

    let mut len: i32 = 0;
    if ((*vt::<ElementArrayVtbl>(array)).get_length)(array, &mut len) != S_OK {
        release(array);
        return out;
    }

    let limit = (len as usize).min(max_elements) as i32;
    for i in 0..limit {
        let mut elem: *mut c_void = null_mut();
        if ((*vt::<ElementArrayVtbl>(array)).get_element)(array, i, &mut elem) != S_OK
            || elem.is_null()
        {
            continue;
        }
        if let Some(b) = read_button(elem, info) {
            out.push(b);
        }
        release(elem);
    }

    release(array);

    if len as usize > max_elements {
        log::debug(&format!(
            "UIA: 要素数 {} が上限 {} を超えたため打ち切りました",
            len, max_elements
        ));
    }

    out
}

/// ウィンドウ配下から OK 相当のボタンを UI Automation で探す。
pub unsafe fn find_ok_button(
    hwnd: HWND,
    extra: &[String],
    max_elements: usize,
    info: &mut ScanInfo,
) -> Option<Found> {
    let mut best: Option<(i32, Found)> = None;
    for b in collect_buttons(hwnd, max_elements, info) {
        let score = label_score(&b.name, extra);
        if score <= 0 {
            continue;
        }
        if best.as_ref().is_none_or(|(s, _)| score > *s) {
            best = Some((
                score,
                Found {
                    rect: b.rect,
                    name: b.name,
                },
            ));
        }
    }
    best.map(|(_, f)| f)
}

/// フォーカス中のボタンと同じ行に「キャンセル」相当のボタンがあるか。
///
/// ダイアログのボタン行にはほぼ必ず取り消し系のボタンが並ぶので、
/// これを「いまダイアログが開いている」ことの手がかりにする。
/// ラベルを個別に登録しなくても既定ボタンを追えるようになる。
pub unsafe fn has_cancel_sibling(hwnd: HWND, focused: &RECT, max_elements: usize) -> bool {
    // UIA の矩形も物理ピクセルなので、しきい値を拡大率に合わせる
    let scale = crate::dialog::dpi_scale_percent(hwnd);
    let mut info = ScanInfo::default();
    for b in collect_buttons(hwnd, max_elements, &mut info) {
        if !crate::dialog::is_cancel_label(&b.name) {
            continue;
        }
        if crate::dialog::same_button_row(&b.rect, focused, scale) {
            return true;
        }
    }
    false
}

/// フォーカス中のボタン。
pub struct FocusedButton {
    pub name: String,
    pub rect: RECT,
    /// ラベルが OK 相当として登録されているものと一致したか
    pub label_matched: bool,
}

/// 現在フォーカスを持っている要素がボタンなら、その情報を返す。
///
/// ウィンドウ内部に描画されるダイアログ (WinUI の ContentDialog など) は
/// 新しい HWND を作らないため、ウィンドウイベントでは検出できない。
/// 一方でダイアログが開くとフォーカスは既定ボタンへ移るので、
/// 「フォーカス中の要素がボタンか」を見れば拾える。
///
/// ツリー走査をせず `GetFocusedElement` 1 回で済むため非常に軽い。
pub unsafe fn focused_button(extra: &[String]) -> Option<FocusedButton> {
    let auto = automation();
    if auto.is_null() {
        return None;
    }

    let mut elem: *mut c_void = null_mut();
    if ((*vt::<AutomationVtbl>(auto)).get_focused_element)(auto, &mut elem) != S_OK
        || elem.is_null()
    {
        return None;
    }

    let mut info = ScanInfo::default();
    let button = read_button(elem, &mut info);
    release(elem);

    let b = button?;
    Some(FocusedButton {
        label_matched: label_score(&b.name, extra) > 0,
        name: b.name,
        rect: b.rect,
    })
}

/// 1 要素を読み取り、押下可能なボタンなら名前と矩形を返す。
unsafe fn read_button(elem: *mut c_void, info: &mut ScanInfo) -> Option<ButtonInfo> {
    let vtbl = vt::<ElementVtbl>(elem);

    let mut control_type: i32 = 0;
    if ((*vtbl).get_current_control_type)(elem, &mut control_type) != S_OK {
        return None;
    }

    let mut offscreen: i32 = 0;
    let visible =
        !(((*vtbl).get_current_is_offscreen)(elem, &mut offscreen) == S_OK && offscreen != 0);

    // 処理中ダイアログの判別。走査は 1 回で済ませたいのでここで拾っておく
    if control_type == CONTROL_TYPE_PROGRESSBAR {
        if visible {
            info.progress = true;
        }
        return None;
    }

    if control_type != CONTROL_TYPE_BUTTON && control_type != CONTROL_TYPE_HYPERLINK {
        return None;
    }

    let mut enabled: i32 = 0;
    if ((*vtbl).get_current_is_enabled)(elem, &mut enabled) != S_OK || enabled == 0 {
        return None;
    }
    if !visible {
        return None;
    }

    let mut name_bstr: *mut u16 = null_mut();
    if ((*vtbl).get_current_name)(elem, &mut name_bstr) != S_OK {
        return None;
    }
    let name = bstr_to_string(name_bstr);
    if name.is_empty() {
        return None;
    }

    // 診断のために名前を控えておく
    if info.seen.len() < 30 && !info.seen.iter().any(|s| s == &name) {
        info.seen.push(name.clone());
    }

    let mut rect: RECT = std::mem::zeroed();
    if ((*vtbl).get_current_bounding_rectangle)(elem, &mut rect) != S_OK {
        return None;
    }
    if rect.right <= rect.left || rect.bottom <= rect.top {
        return None;
    }

    Some(ButtonInfo { name, rect })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// VARIANT のレイアウトが対象プラットフォームと一致していること。
    ///
    /// `CreatePropertyCondition` は VARIANT を値渡しで受け取るため、
    /// 大きさがずれると引数が丸ごと壊れる。x64 で 24 バイト、x86 で 16 バイト。
    #[test]
    fn variant_matches_the_platform_layout() {
        let expected = if cfg!(target_pointer_width = "64") {
            24
        } else {
            16
        };
        assert_eq!(std::mem::size_of::<Variant>(), expected);
        assert_eq!(std::mem::align_of::<Variant>(), 8);
    }
}
