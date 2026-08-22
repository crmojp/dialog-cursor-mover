use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;

/// Rust の &str を NUL 終端の UTF-16 バッファに変換する。
pub fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// UTF-16 バッファ（NUL 終端でもよい）を String に変換する。
pub fn from_wide(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

/// 固定長配列へ NUL 終端で文字列を書き込む（szTip などのフィールド用）。
pub fn fill_wide(dst: &mut [u16], src: &str) {
    if dst.is_empty() {
        return;
    }
    let w: Vec<u16> = OsStr::new(src).encode_wide().collect();
    let mut n = w.len().min(dst.len() - 1);
    // サロゲートペアの途中で切らない。切ると上位サロゲートだけが残り、
    // Windows の描画では豆腐になる
    if n > 0 && (0xD800..=0xDBFF).contains(&w[n - 1]) {
        n -= 1;
    }
    dst[..n].copy_from_slice(&w[..n]);
    dst[n] = 0;
}

/// `%SystemRoot%\System32\<名前>` の絶対パス。
///
/// `ShellExecuteW` も `CreateProcess` も、修飾されていない名前を渡されると
/// システムディレクトリより先にカレントディレクトリを探す。このアプリは
/// 管理者権限で動くことがあり、そのときのカレントディレクトリは exe の
/// 置き場所なので、外部プログラムは必ず絶対パスで起動する。
///
/// 取得に失敗した場合は名前をそのまま返す。従来どおりの探索に戻るだけで、
/// 起動そのものができなくなるよりはよい。
pub fn system32_path(name: &str) -> String {
    let mut buf = [0u16; 260];
    // 成功時は NUL を含まない長さ、バッファ不足なら必要な長さ（NUL 込み）が返る
    let n = unsafe { GetSystemDirectoryW(buf.as_mut_ptr(), buf.len() as u32) };
    if n == 0 || n as usize >= buf.len() {
        return name.to_string();
    }
    format!("{}\\{}", from_wide(&buf[..n as usize]), name)
}
