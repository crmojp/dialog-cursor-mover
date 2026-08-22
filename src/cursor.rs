//! 移動中だけシステムカーソルをテーマの画像に差し替える。
//!
//! `SetSystemCursor` はデスクトップ全体に効く API なので、差し替えっぱなしで
//! プロセスが落ちるとカーソルが戻らなくなる。そのため
//!   * 差し替え中はマーカーファイルを置く
//!   * 起動時にマーカーが残っていれば無条件で復元する
//!   * 終了時・セッション終了時にも復元する
//!
//! という三重の保険をかけている。

use std::cell::{Cell, RefCell};
use std::ptr::null_mut;

use windows_sys::Win32::UI::WindowsAndMessaging::{
    CopyIcon, DestroyIcon, LoadCursorFromFileW, SetSystemCursor, SystemParametersInfoW, HCURSOR,
};

use crate::config::config_dir;
use crate::log;
use crate::util::wide;

/// 通常の矢印カーソル (OCR_NORMAL)
const OCR_NORMAL: u32 = 32512;
const SPI_SETCURSORS: u32 = 0x0057;

/// 走行速度。カーソルのコマ送り速度の選択に使う。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Speed {
    Fast = 0,
    Normal = 1,
    Slow = 2,
}

impl Speed {
    /// 移動時間から速度段階を決める。INI を直接編集した場合も自然に対応できる。
    pub fn from_duration_ms(ms: u32) -> Speed {
        if ms <= 220 {
            Speed::Fast
        } else if ms <= 450 {
            Speed::Normal
        } else {
            Speed::Slow
        }
    }

    fn suffix(self) -> &'static str {
        match self {
            // 「普通」は無印。速度別ファイルが無くてもここへ落ちてくる
            Speed::Fast => "_fast",
            Speed::Normal => "",
            Speed::Slow => "_slow",
        }
    }
}

thread_local! {
    /// [右速い, 右普通, 右遅い, 左速い, 左普通, 左遅い] のハンドルキャッシュ
    static CURSORS: Cell<[usize; 6]> = const { Cell::new([0; 6]) };
    /// キャッシュしたカーソルのテーマ名。
    /// テーマを切り替えたら読み直す必要がある。読み込みに失敗したスロットは
    /// usize::MAX を記録するため、これがないと切り替えても再試行されない。
    static CACHED_THEME: RefCell<Option<String>> = const { RefCell::new(None) };
    static ACTIVE: Cell<bool> = const { Cell::new(false) };
    /// 現在適用中のスロット番号。同じなら再設定しない
    static CUR_SLOT: Cell<i8> = const { Cell::new(-1) };
}

fn marker_path() -> std::path::PathBuf {
    config_dir().join(".cursor_override")
}

fn slot_of(dir_right: bool, speed: Speed) -> usize {
    (if dir_right { 0 } else { 3 }) + speed as usize
}

/// カーソルファイルのパス候補を優先順で返す。
///
/// テーマの `theme\<名前>\cursor_*.ani` だけを見る。
/// 既定では Windows のカーソルをそのまま使うため、テーマが無ければ候補も無い。
fn candidates(theme: &str, dir_right: bool, speed: Speed) -> Vec<std::path::PathBuf> {
    crate::theme::cursor(theme, dir_right, speed.suffix())
        .into_iter()
        .collect()
}

/// カーソルファイルを読み込む（結果はキャッシュする）。
unsafe fn load(theme: &str, dir_right: bool, speed: Speed) -> HCURSOR {
    // テーマが変わったらキャッシュを捨てる。
    // 失敗を記録したスロットも一緒に消えるので、新しいテーマで再試行される。
    let changed = CACHED_THEME.with(|c| {
        let mut slot = c.borrow_mut();
        if slot.as_deref() != Some(theme) {
            *slot = Some(theme.to_string());
            true
        } else {
            false
        }
    });
    if changed {
        // 自分で読み込んだハンドルなので、捨てる前に破棄する。
        // SetSystemCursor に渡すのは CopyIcon した複製なので、
        // 適用中のテーマのものであってもここで解放してよい。
        let old = CURSORS.with(|c| c.replace([0; 6]));
        for h in old {
            if h != 0 && h != usize::MAX {
                DestroyIcon(h as HCURSOR);
            }
        }
    }

    let slot = slot_of(dir_right, speed);
    let cached = CURSORS.with(|c| c.get()[slot]);
    if cached == usize::MAX {
        return null_mut();
    }
    if cached != 0 {
        return cached as HCURSOR;
    }

    for path in candidates(theme, dir_right, speed) {
        if !path.is_file() {
            continue;
        }
        let w = wide(&path.to_string_lossy());
        let h = LoadCursorFromFileW(w.as_ptr());
        if h.is_null() {
            log::info(&format!(
                "カーソルの読み込みに失敗しました: {}",
                path.display()
            ));
            continue;
        }
        CURSORS.with(|c| {
            let mut arr = c.get();
            arr[slot] = h as usize;
            c.set(arr);
        });
        return h;
    }

    // どこを探したかを残す。テーマの置き場所を間違えたときに気づけるようにする
    let tried: Vec<String> = candidates(theme, dir_right, speed)
        .iter()
        .map(|p| p.display().to_string())
        .collect();
    log::debug(&format!(
        "カーソル: ファイルが見つかりません 探索先: {}",
        if tried.is_empty() {
            "(テーマ未指定)".to_string()
        } else {
            tried.join(" / ")
        }
    ));
    CURSORS.with(|c| {
        let mut arr = c.get();
        arr[slot] = usize::MAX;
        c.set(arr);
    });
    null_mut()
}

/// テーマのカーソルを適用する。向き・速度が同じなら何もしない。
pub unsafe fn set_running(theme: &str, dir_right: bool, speed: Speed) -> bool {
    let slot = slot_of(dir_right, speed) as i8;
    if ACTIVE.with(|c| c.get()) && CUR_SLOT.with(|c| c.get()) == slot {
        return true;
    }

    let src = load(theme, dir_right, speed);
    if src.is_null() {
        return false;
    }

    // SetSystemCursor はハンドルの所有権を奪って破棄するため、必ず複製を渡す
    let copy = CopyIcon(src);
    if copy.is_null() {
        return false;
    }
    if SetSystemCursor(copy, OCR_NORMAL) == 0 {
        // ここで DestroyIcon を呼ばない。
        //
        // SetSystemCursor は成功時にハンドルの所有権を奪って破棄するが、
        // 失敗時に消費するかどうかは文書化されていない。既に消費されていた
        // 場合、ここでの破棄は USER オブジェクトの二重解放になる。
        // この分岐はカーソル差し替えがそもそも機能していない状況でしか
        // 通らないので、ハンドル 1 個の漏れを受け入れるほうが安全側。
        return false;
    }

    if !ACTIVE.with(|c| c.get()) {
        // この目印は、異常終了でカーソルが戻らなかったときの復旧に使う。
        // 書けないと保険が効かなくなるので、黙って見逃さない
        if let Err(e) = std::fs::write(marker_path(), b"1") {
            log::info(&format!(
                "カーソル: 復旧用の目印を書けません（異常終了時に戻らない可能性があります）: {e}"
            ));
        }
    }
    ACTIVE.with(|c| c.set(true));
    CUR_SLOT.with(|c| c.set(slot));
    true
}

/// 元のカーソルに戻す。
pub unsafe fn restore() {
    if !ACTIVE.with(|c| c.get()) {
        return;
    }
    // ユーザーが設定しているカーソルスキームを読み直させる。
    //
    // 最後の引数に SPIF_SENDCHANGE は渡さない。渡すとデスクトップ上の
    // 全トップレベルウィンドウへ WM_SETTINGCHANGE が配信される。
    // カーソルを戻すのは移動のたびなので、数秒に一度、無関係なアプリ
    // すべてに通知を撒くことになる。これを受けてレイアウトを組み直す
    // 実装のアプリがあると、そちらの表示や重なり順が乱れうる。
    //
    // システムカーソルの復元はこの呼び出し自体が行うので、通知は要らない。
    SystemParametersInfoW(SPI_SETCURSORS, 0, null_mut(), 0);
    ACTIVE.with(|c| c.set(false));
    CUR_SLOT.with(|c| c.set(-1));
    let _ = std::fs::remove_file(marker_path());
}

#[allow(dead_code)]
pub fn is_active() -> bool {
    ACTIVE.with(|c| c.get())
}

/// 起動時に呼ぶ。前回異常終了してカーソルが戻っていない場合に復元する。
pub unsafe fn restore_stale_override() {
    let marker = marker_path();
    if !marker.exists() {
        return;
    }
    SystemParametersInfoW(SPI_SETCURSORS, 0, null_mut(), 0);
    let _ = std::fs::remove_file(&marker);
    log::info("前回終了時に戻っていなかったカーソルを復元しました");
}
