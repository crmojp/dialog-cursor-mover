//! テーマ（アイコン・カーソル・音の差し替え）の解決。
//!
//! exe と同じ階層の `theme\<名前>\` を 1 つのテーマとして扱う。
//! 含められるファイルは次のとおりで、いずれも任意。無いものは既定
//! （内蔵アイコン、Windows のカーソルと音）にフォールバックする。
//!
//! ```text
//! theme\
//!   cat\
//!     icon.ico
//!     cursor_right.ani  cursor_right_fast.ani  cursor_right_slow.ani
//!     cursor_left.ani   cursor_left_fast.ani   cursor_left_slow.ani
//!     sound.wav
//! ```
//!
//! テーマ名は設定ファイルから来てパスの一部になるため、
//! 言語コードと同じ基準で検証してから使う。

use std::path::PathBuf;

use crate::config::exe_dir;

/// テーマを置くディレクトリ名。
///
/// リポジトリの `assets\`（生成スクリプトと素材の置き場）とは別物で、
/// こちらは配布物に含める実行時の参照先。
pub const THEMES_DIR: &str = "theme";

/// テーマ名として安全か。
///
/// `Path::join` は絶対パスを渡されると基準ディレクトリを置き換えてしまうため、
/// 英数字・ハイフン・アンダースコアだけを許可する。
pub fn is_valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// テーマのディレクトリ。名前が空か不正なら None（＝既定を使う）。
pub fn dir_of(name: &str) -> Option<PathBuf> {
    if !is_valid_name(name) {
        return None;
    }
    let dir = exe_dir().join(THEMES_DIR).join(name);
    dir.is_dir().then_some(dir)
}

/// テーマのファイル 1 つあたりの上限。
///
/// アイコンもカーソルも音も、この用途では数百 KB あれば足りる。
/// 巨大なファイルを Windows のローダに渡さないための歯止め。
const MAX_ASSET_BYTES: u64 = 8 * 1024 * 1024;

/// テーマ内のファイルを探す。存在しなければ None。
///
/// テーマは第三者が作ったものを持ち込むことが想定されるため、
/// 次の条件を満たすものだけを返す。
///
/// * 通常のファイルであること（シンボリックリンクやジャンクションを追わない）
/// * `theme\<名前>\` の直下にあり、そこから外へ出ていないこと
/// * サイズが上限以内であること
pub fn file_of(name: &str, file: &str) -> Option<PathBuf> {
    let dir = dir_of(name)?;
    let path = dir.join(file);

    // symlink_metadata はリンク自体を見る。metadata だとリンク先を追ってしまい、
    // theme ディレクトリの外にあるファイルを読み込めてしまう。
    // ファイルが無いのは普通のこと（テーマは全ファイルが任意）なので黙って諦める。
    // 一方、「存在するのに受け入れなかった」場合は理由を残す。
    // 拒否は安全のための判断なので、黙って既定に戻ると原因が追えない。
    let meta = std::fs::symlink_metadata(&path).ok()?;
    if !meta.is_file() {
        crate::log::info(&format!(
            "テーマ: 通常のファイルではないので使いません（リンクの可能性） theme=\"{name}\" file=\"{file}\""
        ));
        return None;
    }
    if meta.len() > MAX_ASSET_BYTES {
        crate::log::info(&format!(
            "テーマ: ファイルが大きすぎます（上限 {} MB） theme=\"{name}\" file=\"{file}\" size={}",
            MAX_ASSET_BYTES / 1024 / 1024,
            meta.len()
        ));
        return None;
    }

    // 実体パスがテーマのディレクトリ配下に留まることを確かめる。
    // ジャンクションなど symlink_metadata では見抜けない経路への保険。
    let real = std::fs::canonicalize(&path).ok()?;
    let real_dir = std::fs::canonicalize(&dir).ok()?;
    if !real.starts_with(&real_dir) {
        crate::log::info(&format!(
            "警告: テーマの参照がテーマフォルダーの外を指しています theme=\"{name}\" file=\"{file}\""
        ));
        return None;
    }

    Some(path)
}

/// アイコンのパス。
pub fn icon(name: &str) -> Option<PathBuf> {
    file_of(name, "icon.ico")
}

/// 音のパス。
pub fn sound(name: &str) -> Option<PathBuf> {
    file_of(name, "sound.wav")
}

/// 走行カーソルのパス。速度別が無ければ無印にフォールバックする。
pub fn cursor(name: &str, dir_right: bool, speed_suffix: &str) -> Option<PathBuf> {
    let side = if dir_right { "right" } else { "left" };
    file_of(name, &format!("cursor_{side}{speed_suffix}.ani"))
        .or_else(|| file_of(name, &format!("cursor_{side}.ani")))
}

/// 利用できるテーマの一覧。
///
/// `theme\` 直下のディレクトリのうち、認識できるファイルを
/// 1 つ以上持つものだけを返す。
pub fn available() -> Vec<String> {
    let mut names = Vec::new();
    let Ok(entries) = std::fs::read_dir(exe_dir().join(THEMES_DIR)) else {
        return names;
    };

    for entry in entries.flatten() {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(|s| s.to_string()) else {
            continue;
        };
        if !is_valid_name(&name) {
            continue;
        }
        // 中身が空のディレクトリは一覧に出さない
        let has_content =
            icon(&name).is_some() || sound(&name).is_some() || cursor(&name, true, "").is_some();
        if has_content {
            names.push(name);
        }
        if names.len() >= 32 {
            break;
        }
    }
    names.sort();
    names
}
