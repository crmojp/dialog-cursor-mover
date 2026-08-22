//! 到達地点に同心円のアニメーションを表示する。
//!
//! クリックを透過するレイヤードウィンドウを一時的に作り、
//! 外側から中心へ縮んでいくリングを描いてから閉じる。
//!
//! * `WS_EX_TRANSPARENT` — マウス操作を下のウィンドウへ素通しする
//! * `WS_EX_NOACTIVATE` — 表示してもフォーカスを奪わない
//! * `WS_EX_TOOLWINDOW` — Alt+Tab やタスクバーに出さない
//!
//! 描画は `UpdateLayeredWindow` にピクセル単位のアルファを渡す方式で行う。
//! GDI の描画命令ではアンチエイリアスがかからず、縮小するリングの縁が
//! 目に見えて粗くなるため、ピクセルを直接組み立てている。

use std::cell::Cell;
use std::ffi::c_void;
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{COLORREF, HWND, POINT, SIZE};
use windows_sys::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject,
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, KillTimer, RegisterClassW, SetTimer, ShowWindow,
    UpdateLayeredWindow, SW_SHOWNA, ULW_ALPHA, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

use crate::log;
use crate::util::wide;

/// アニメーションの更新間隔（約 60fps）
const FRAME_MS: u32 = 16;
/// 同時に描くリングの本数
const RING_COUNT: usize = 3;
/// リングの線幅（px）
const RING_WIDTH: f64 = 3.0;
/// タイマー ID
const TIMER_ID: usize = 1;

thread_local! {
    /// 表示中のウィンドウ（0 = なし）
    static WINDOW: Cell<usize> = const { Cell::new(0) };
    /// 開始からの経過フレーム数
    static FRAME: Cell<u32> = const { Cell::new(0) };
    /// 1 回のアニメーションの長さ（フレーム数）
    static TOTAL_FRAMES: Cell<u32> = const { Cell::new(0) };
    /// 描画サイズ（= 最大直径）
    static EXTENT: Cell<i32> = const { Cell::new(0) };
    /// リングの色（R, G, B）
    static COLOR: Cell<(u8, u8, u8)> = const { Cell::new((58, 132, 214)) };
    /// ウィンドウクラスを登録済みか
    static REGISTERED: Cell<bool> = const { Cell::new(false) };
}

/// リップル用ウィンドウのクラス名。
/// 検出側でこの名前を照合し、自分が出した表示を走査対象から外す。
pub const CLASS_NAME: &str = "DialogCursorMoverRipple";

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wp: usize, lp: isize) -> isize {
    use windows_sys::Win32::UI::WindowsAndMessaging::{DefWindowProcW, WM_TIMER};
    if msg == WM_TIMER && wp == TIMER_ID {
        on_frame(hwnd);
        return 0;
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}

unsafe fn ensure_class() -> bool {
    if REGISTERED.with(|c| c.get()) {
        return true;
    }
    let name = wide(CLASS_NAME);
    let mut wc: WNDCLASSW = std::mem::zeroed();
    wc.lpfnWndProc = Some(wnd_proc);
    wc.hInstance = GetModuleHandleW(std::ptr::null());
    wc.lpszClassName = name.as_ptr();
    let ok = RegisterClassW(&wc) != 0;
    REGISTERED.with(|c| c.set(ok));
    if !ok {
        log::info("リップル表示用のウィンドウクラスを登録できませんでした");
    }
    ok
}

/// 到達地点でアニメーションを開始する。
///
/// 既に表示中なら、そちらを打ち切ってから開始する。
pub unsafe fn play(center_x: i32, center_y: i32, size: u32, duration_ms: u32, color: u32) {
    stop();

    if size == 0 || duration_ms == 0 || !ensure_class() {
        return;
    }

    let extent = size as i32;
    let half = extent / 2;

    let ex_style =
        WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST;
    let name = wide(CLASS_NAME);
    let hwnd = CreateWindowExW(
        ex_style,
        name.as_ptr(),
        std::ptr::null(),
        WS_POPUP,
        center_x - half,
        center_y - half,
        extent,
        extent,
        null_mut(),
        null_mut(),
        GetModuleHandleW(std::ptr::null()),
        null_mut(),
    );
    if hwnd.is_null() {
        log::info("リップル: ウィンドウを作成できませんでした");
        return;
    }

    // COLORREF は 0x00BBGGRR。設定では見慣れた 0xRRGGBB で受け取る
    let r = ((color >> 16) & 0xFF) as u8;
    let g = ((color >> 8) & 0xFF) as u8;
    let b = (color & 0xFF) as u8;

    WINDOW.with(|c| c.set(hwnd as usize));
    FRAME.with(|c| c.set(0));
    EXTENT.with(|c| c.set(extent));
    COLOR.with(|c| c.set((r, g, b)));
    TOTAL_FRAMES.with(|c| c.set((duration_ms / FRAME_MS).max(1)));

    // 中身を描いてから表示する。
    // WS_POPUP だけでは非表示のままなので、ここで明示的に見せる必要がある。
    // SW_SHOWNA はフォーカスを奪わずに表示する指定。
    on_frame(hwnd);
    // on_frame は描画に失敗すると stop() を呼び、この hwnd を破棄する。
    // 破棄済みのウィンドウへ ShowWindow / SetTimer を投げない
    if WINDOW.with(|c| c.get()) != hwnd as usize {
        return;
    }
    ShowWindow(hwnd, SW_SHOWNA);
    SetTimer(hwnd, TIMER_ID, FRAME_MS, None);

    log::debug(&format!(
        "リップル: 開始 ({}, {}) size={} {}ms",
        center_x, center_y, size, duration_ms
    ));
}

/// 表示中のアニメーションを閉じる。
pub unsafe fn stop() {
    let hwnd = WINDOW.with(|c| c.replace(0));
    if hwnd != 0 {
        KillTimer(hwnd as HWND, TIMER_ID);
        DestroyWindow(hwnd as HWND);
    }
}

unsafe fn on_frame(hwnd: HWND) {
    let frame = FRAME.with(|c| {
        let n = c.get() + 1;
        c.set(n);
        n
    });
    let total = TOTAL_FRAMES.with(|c| c.get());

    if frame > total {
        stop();
        return;
    }

    let t = (frame as f64 / total as f64).clamp(0.0, 1.0);
    if !draw(hwnd, t) {
        log::info("リップル: 描画に失敗したため中止しました");
        stop();
    }
}

/// 進捗 `t`（0.0〜1.0）の 1 フレームを描く。
unsafe fn draw(hwnd: HWND, t: f64) -> bool {
    let extent = EXTENT.with(|c| c.get());
    if extent <= 0 {
        return false;
    }
    let (cr, cg, cb) = COLOR.with(|c| c.get());

    let screen = GetDC(null_mut());
    if screen.is_null() {
        return false;
    }
    let mem = CreateCompatibleDC(screen);
    if mem.is_null() {
        ReleaseDC(null_mut(), screen);
        return false;
    }

    // トップダウンの 32bpp DIB。負の高さで上から下の並びにする
    let mut bmi: BITMAPINFO = std::mem::zeroed();
    bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    bmi.bmiHeader.biWidth = extent;
    bmi.bmiHeader.biHeight = -extent;
    bmi.bmiHeader.biPlanes = 1;
    bmi.bmiHeader.biBitCount = 32;
    bmi.bmiHeader.biCompression = BI_RGB;

    let mut bits: *mut c_void = null_mut();
    let bitmap: HBITMAP = CreateDIBSection(mem, &bmi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
    if bitmap.is_null() || bits.is_null() {
        // ビットマップだけ作られて bits が返らない場合に備え、
        // 非 null なら破棄してから抜ける。ここを落とすと GDI オブジェクトが残る。
        if !bitmap.is_null() {
            DeleteObject(bitmap as _);
        }
        DeleteDC(mem);
        ReleaseDC(null_mut(), screen);
        return false;
    }
    let old = SelectObject(mem, bitmap as _);

    render_pixels(bits as *mut u8, extent, t, (cr, cg, cb));

    // どちらも入力としてしか使われないので不変で渡す
    let src_pos = POINT { x: 0, y: 0 };
    let size = SIZE {
        cx: extent,
        cy: extent,
    };
    let blend_struct = BlendFunction {
        blend_op: 0, // AC_SRC_OVER
        blend_flags: 0,
        source_constant_alpha: 255,
        alpha_format: 1, // AC_SRC_ALPHA
    };

    let ok = UpdateLayeredWindow(
        hwnd,
        screen,
        null_mut(),
        &size,
        mem,
        &src_pos,
        0 as COLORREF,
        &blend_struct as *const BlendFunction as *const _,
        ULW_ALPHA,
    ) != 0;

    SelectObject(mem, old);
    DeleteObject(bitmap as _);
    DeleteDC(mem);
    ReleaseDC(null_mut(), screen);

    ok
}

/// `BLENDFUNCTION` と同じレイアウト。
#[repr(C)]
struct BlendFunction {
    blend_op: u8,
    blend_flags: u8,
    source_constant_alpha: u8,
    alpha_format: u8,
}

/// リングを 1 枚分ピクセルへ書き込む。
///
/// `UpdateLayeredWindow` は乗算済みアルファを要求するため、
/// 各チャンネルにアルファを掛けてから格納する。
fn render_pixels(bits: *mut u8, extent: i32, t: f64, (cr, cg, cb): (u8, u8, u8)) {
    let n = (extent * extent) as usize * 4;
    let buf = unsafe { std::slice::from_raw_parts_mut(bits, n) };
    buf.fill(0);

    let center = extent as f64 / 2.0;
    let max_radius = center - RING_WIDTH;

    // 外側から中心へ縮む。終盤ほど薄くする
    for i in 0..RING_COUNT {
        // リングごとに位相をずらし、続けて縮んでいくように見せる
        let offset = i as f64 / RING_COUNT as f64;
        let phase = t + offset;
        if phase > 1.0 {
            continue;
        }
        // 半径は外側から中心へ。1.0 で 0 になる
        let radius = max_radius * (1.0 - phase);
        if radius <= 0.5 {
            continue;
        }
        // 手前のリングほど濃く、全体としては終盤に消える
        let alpha = (1.0 - phase).powf(0.7) * (1.0 - offset * 0.5);
        if alpha <= 0.01 {
            continue;
        }
        stroke_circle(buf, extent, center, radius, alpha, (cr, cg, cb));
    }
}

/// アンチエイリアスをかけた円周を 1 本描く。
fn stroke_circle(
    buf: &mut [u8],
    extent: i32,
    center: f64,
    radius: f64,
    alpha: f64,
    (cr, cg, cb): (u8, u8, u8),
) {
    let half_w = RING_WIDTH / 2.0;
    // リングが影響する範囲は細い円環なので、外接する正方形ではなく
    // 各行で実際に交わる区間だけを走査する。外接正方形だと 1 リングあたり
    // 直径の 2 乗ぶんの画素を毎フレーム舐めることになり、
    // ripple_size を上限まで上げたときに CPU を使い切ってしまう。
    let outer = radius + half_w + 1.0;
    let inner = radius - half_w - 1.0;

    let y_lo = ((center - outer).floor().max(0.0)) as i32;
    let y_hi = ((center + outer).ceil().min(extent as f64)) as i32;

    for y in y_lo..y_hi {
        let dy = y as f64 + 0.5 - center;
        let dy2 = dy * dy;
        if dy2 >= outer * outer {
            continue;
        }
        let x_outer = (outer * outer - dy2).sqrt();
        // 内側の穴を跨ぐ行は、左右 2 つの区間に分かれる
        let x_inner = if inner > 0.0 && dy2 < inner * inner {
            (inner * inner - dy2).sqrt()
        } else {
            0.0
        };
        let spans = if x_inner > 0.0 {
            [(-x_outer, -x_inner), (x_inner, x_outer)]
        } else {
            [(-x_outer, x_outer), (0.0, 0.0)]
        };

        for (from, to) in spans {
            if to <= from {
                continue;
            }
            let x_lo = ((center + from).floor().max(0.0)) as i32;
            let x_hi = ((center + to).ceil().min(extent as f64)) as i32;
            for x in x_lo..x_hi {
                let dx = x as f64 + 0.5 - center;
                let dist = (dx * dx + dy2).sqrt();
                // 線の中心からの距離で不透明度を決める
                let edge = (half_w - (dist - radius).abs()).clamp(0.0, 1.0);
                if edge <= 0.0 {
                    continue;
                }
                let a = alpha * edge;
                if a <= 0.004 {
                    continue;
                }

                let idx = ((y * extent + x) as usize) * 4;
                let existing = buf[idx + 3] as f64 / 255.0;
                // 既に描かれたリングと重なる場合は濃いほうを残す
                let merged = existing.max(a);
                let m = merged.min(1.0);

                // 乗算済みアルファ (BGRA の順)
                buf[idx] = (cb as f64 * m) as u8;
                buf[idx + 1] = (cg as f64 * m) as u8;
                buf[idx + 2] = (cr as f64 * m) as u8;
                buf[idx + 3] = (m * 255.0) as u8;
            }
        }
    }
}
