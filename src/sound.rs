use std::ptr::null_mut;

use windows_sys::Win32::Media::Audio::PlaySoundW;

use crate::util::wide;

const SND_ASYNC: u32 = 0x0000_0001;
const SND_NODEFAULT: u32 = 0x0000_0002;
const SND_FILENAME: u32 = 0x0002_0000;
const SND_NOSTOP: u32 = 0x0000_0010;

/// 再生を許可する wav の上限サイズ。
///
/// 巨大なファイルを指定されても、音声デコーダに丸ごと読み込ませない。
const MAX_WAV_BYTES: u64 = 16 * 1024 * 1024;

/// 指定されたパスを再生対象として受け入れてよいか。
///
/// `wav_path` は config.ini で任意のパスを指定できる唯一のアセットなので、
/// ここだけは経路を絞る。他のアセット (アイコン・カーソル) はファイル名が
/// 固定で exe と同じディレクトリしか見ないため、この検査は不要。
fn is_acceptable(path: &str) -> Result<(), &'static str> {
    // UNC パスを拒否する。
    // \\host\share を指定されると、再生のたびにリモートへ接続しに行く。
    // 攻撃者が用意したホストを指定させられると、認証情報が飛ぶ恐れがある。
    if path.starts_with("\\\\") || path.starts_with("//") {
        return Err("UNC パスは再生できません");
    }

    let p = std::path::Path::new(path);

    // 拡張子を .wav に限定する。
    // PlaySoundW は中身で形式を判定するため、拡張子だけでは安全にならないが、
    // 設定ミスや意図しないファイルの指定を早い段階で弾ける。
    let ext_ok = p
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("wav"))
        .unwrap_or(false);
    if !ext_ok {
        return Err("拡張子が .wav ではありません");
    }

    let meta = std::fs::metadata(p).map_err(|_| "ファイルが見つかりません")?;
    if !meta.is_file() {
        return Err("通常のファイルではありません");
    }
    if meta.len() > MAX_WAV_BYTES {
        return Err("ファイルが大きすぎます");
    }
    Ok(())
}

/// 指定した .wav を非同期再生する。受け入れられないパスなら何もしない。
pub fn play_wav(path: &str) -> bool {
    let path = path.trim();
    if path.is_empty() {
        return false;
    }
    if let Err(reason) = is_acceptable(path) {
        crate::log::debug(&format!(
            "サウンド: 再生しません ({reason}) path=\"{path}\""
        ));
        return false;
    }

    let w = wide(path);
    // SND_NOSTOP: 再生中の音を強制的に止めない
    // SND_NODEFAULT: 失敗時にシステム既定音へフォールバックしない
    unsafe {
        PlaySoundW(
            w.as_ptr(),
            null_mut(),
            SND_FILENAME | SND_ASYNC | SND_NODEFAULT | SND_NOSTOP,
        ) != 0
    }
}

/// 再生中の音を停止する（終了時のクリーンアップ用）。
pub fn stop() {
    unsafe {
        PlaySoundW(std::ptr::null(), null_mut(), 0);
    }
}
