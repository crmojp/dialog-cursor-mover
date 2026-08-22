//! 言語ファイルによる多言語対応。
//!
//! `exe_dir()/lang/<code>.ini` を読み込み、キーに対応する文字列を返す。
//! 見つからないキーは英語の組み込み文字列にフォールバックするため、
//! 言語ファイルが欠けていても、翻訳が一部だけでも動作する。

use std::collections::HashMap;
use std::sync::RwLock;

use crate::config::exe_dir;

/// 既定の言語コード
pub const DEFAULT_LANG: &str = "en";

/// 読み込み済みの翻訳表
static STRINGS: RwLock<Option<HashMap<String, String>>> = RwLock::new(None);

/// 組み込みの英語文字列。言語ファイルが無い、あるいはキーが欠けている場合に使う。
///
/// 言語ファイルのキーはここに列挙したものがすべてである。
const FALLBACK: &[(&str, &str)] = &[
    // --- トレイメニュー ---
    ("menu.enabled", "Enabled (watch dialogs)"),
    ("menu.delay", "Delay"),
    ("menu.delay.current", "Current: {0} ms"),
    ("menu.sound", "Play sound"),
    ("menu.sound.current", "  Current sound: {0}"),
    ("menu.sound.choose", "Choose another .wav..."),
    ("menu.sound.test", "Test playback"),
    ("menu.skip_file_dialogs", "Ignore file/folder pickers"),
    ("menu.move_animation", "Run like a mouse"),
    ("menu.move_speed", "Run speed"),
    ("menu.speed.fast", "Fast"),
    ("menu.speed.normal", "Normal"),
    ("menu.speed.slow", "Slow"),
    ("menu.speed.item", "{0} ({1} ms)"),
    ("menu.cursor_animation", "Use mouse cursor while running"),
    ("menu.autostart", "Start at logon as administrator"),
    ("menu.diagnostics", "Diagnostics"),
    ("menu.test_dialog", "Show test dialog"),
    ("menu.log.off", "Log: off"),
    ("menu.log.normal", "Log: normal"),
    ("menu.log.verbose", "Log: verbose"),
    ("menu.open_log", "Open log"),
    ("menu.open_config", "Open config file"),
    ("menu.reload_config", "Reload settings"),
    ("menu.about", "About"),
    ("menu.exit", "Exit"),
    ("menu.language", "Language"),
    // --- ダイアログ ---
    ("msg.already_running", "DialogCursorMover is already running."),
    ("msg.wav_failed", "Could not play the selected .wav file. Please check the path."),
    (
        "msg.test_dialog",
        "This is a test dialog.\nIf the cursor moves to this OK button after the configured delay, everything works.",
    ),
    (
        "msg.about",
        "DialogCursorMover {0}\n\nMoves the mouse cursor to the OK button\nwhen a dialog appears.\n\nSettings and logs:\n{1}",
    ),
    ("msg.task.installed", "Registered the autostart task.\n\nTask name: {0}"),
    ("msg.task.removed", "Removed the autostart task.\n\nTask name: {0}"),
    ("msg.task.install_failed", "Failed to register the autostart task.\n\n{0}"),
    ("msg.task.remove_failed", "Failed to remove the autostart task.\n\n{0}"),
    ("msg.task.denied", "Administrator rights were denied, so the operation was cancelled."),
    (
        "msg.task.unsafe_location",
        "DialogCursorMover is in a folder that users other than administrators can write to:\n\n{0}\n\nIf you register it to start at logon as administrator, anything placed in that folder later will also run with administrator rights.\n\nMoving it somewhere only administrators can write to is safer.\n\nRegister anyway?",
    ),
    (
        "msg.task.elevated",
        "The operation ran with administrator rights.\n\nThe result is shown in a separate dialog.\nThe menu checkmark updates the next time you open the menu.",
    ),
    ("msg.class_failed", "Failed to register the window class."),
    ("msg.window_failed", "Failed to create the window."),
    ("msg.choose_wav", "Select a .wav file to play"),
    ("msg.filter_wav", "WAV files (*.wav)"),
    ("msg.filter_all", "All files (*.*)"),
    ("msg.log_empty", "(no log entries yet)"),
    ("msg.sound_unset", "(not set)"),
    // --- トレイのツールチップ ---
    ("tip.status", "DialogCursorMover — {0} / delay {1}ms / sound {2}"),
    ("tip.on", "on"),
    ("tip.off", "off"),



    // --- config.ini のコメント（言語ファイルが無い場合の既定）---
    ("cfg.header", "DialogCursorMover settings (UTF-8)\nAfter editing this file by hand, choose \"Reload settings\" from the tray menu."),
    ("cfg.language", "Display language. Matches a file name in the lang directory.\nExample: language = en  ->  lang\\\\en.ini"),
    ("cfg.enabled", "Whether dialog watching is active."),
    ("cfg.delay_ms", "Delay from detecting a dialog until the cursor moves (milliseconds, 0-60000).\nToo small a value may move the cursor before the buttons are laid out.\n200-500 is recommended."),
    ("cfg.sound_enabled", "Whether to play a .wav file when the cursor moves."),
    ("cfg.wav_path", "Full path of the .wav file to play."),
    ("cfg.standard_dialog_only", "true: only standard dialogs (window class #32770) are targeted.\nfalse: popup windows in general are targeted too (more false positives)."),
    ("cfg.require_foreground", "true: only move the cursor while the dialog is actually in the foreground."),
    ("cfg.skip_if_cursor_inside", "true: do nothing when the cursor is already over the button."),
    ("cfg.move_once_per_dialog", "true: react only once per dialog.\n\nSwitching tabs in a settings dialog fires show events for the new page, so\nwithout this the cursor would jump back to OK on every tab change.\nThe record is cleared when the window closes, so reopening it reacts again."),
    ("cfg.ignore_own_process", "true: ignore dialogs raised by this application itself.\nWith false (the default) it also reacts to its own About dialog.\nIn either case it is suppressed while the .wav picker is open."),
    ("cfg.move_animation", "true: glide the cursor to the button instead of jumping."),
    ("cfg.move_duration_ms", "Time taken to run (milliseconds, 0-5000). 0 means jump instantly."),
    ("cfg.move_wobble", "Sideways wobble while running (pixels, 0-40). 0 moves in a straight line."),
    ("cfg.cursor_animation", "true: swap the cursor image for the theme's while moving.
Requires theme\\<name>\\cursor_right.ani and cursor_left.ani.
Note: this temporarily changes the cursor for the whole desktop."),
    ("cfg.abort_on_user_move", "true: cancel the movement if the user moves the mouse while it runs."),
    ("cfg.user_move_threshold", "Cursor offset treated as user input (pixels, 2-500).\nIncrease this if movement is cancelled by false detection."),
    ("cfg.watch_title_changes", "true: treat window title changes as a detection trigger.\n\nExplorer's \"Replace or Skip Files\" dialog appears by reusing the copy progress\nwindow, so no show event is fired. Watching title changes catches it."),
    ("cfg.watch_focus_changes", "true: treat focus changes as a detection trigger.

Dialogs drawn inside an application's own window create no new window,
so they cannot be found by watching for windows. Focus moves to the
default button when such a dialog opens, and that is the only clue.
(only effective when uia_enabled = true)"),
    ("cfg.follow_dialog_default_button", "true: follow the default button even when its label is not registered.

A default button is accepted when a Cancel-like button sits beside it,
or when it sits in the bottom quarter of the dialog. This reaches
wizard 'Next' buttons and lone 'Close' buttons without listing labels.

Note: an application window whose layout resembles a dialog can match
too. Use exclude_processes to rule one out."),
    ("cfg.follow_default_in_browser", "true: apply the default-button rule inside browsers as well.\n\nWeb pages can build arbitrary UI, so a \"Block / Cancel\" layout cannot be told\napart from an application dialog. The default is false so that layout-based\ndetection is skipped inside browsers.\n(Labels matching OK / Yes and so on still work regardless of this setting.)"),
    ("cfg.uia_enabled", "true: also use UI Automation so XAML / WinUI dialogs are supported
(such as the save prompts in newer built-in Windows applications)."),
    ("cfg.uia_dialog_like_only", "true: restrict UI Automation scanning to dialog-shaped windows\n(not resizable, no minimize/maximize box, small relative to the screen).\nSetting this to false also scans browsers and is very slow."),
    ("cfg.uia_max_elements", "Upper limit of elements inspected during a UI Automation scan (20-5000)."),
    ("cfg.log_level", "Diagnostic log: 0 = off, 1 = normal, 2 = verbose (written to log.txt)."),
    ("cfg.skip_file_dialogs", "true: ignore file and folder pickers (Open, Save As, Select Folder).\nTheir confirm buttons use the IDOK control id, so disabling this makes the\napplication react to them."),
    ("cfg.skip_progress_dialogs", "true: ignore \"in progress\" dialogs that contain a progress bar\n(copy and extraction progress windows, for example)."),
    ("cfg.exclude_processes", "Ignore dialogs raised by these executables.
(comma separated, case insensitive)
Example: exclude_processes = installer.exe, backup.exe"),
    ("cfg.exclude_titles", "Ignore dialogs whose title contains any of these strings.
(comma separated, substring match, case insensitive)
Example: exclude_titles = Print, Update available"),
    ("cfg.extra_button_labels", "Additional button labels treated as an OK button (comma separated).\n\nOnly OK / Yes / Continue and their Japanese equivalents are recognised by\ndefault. Add labels here to move the cursor to other buttons.\n\nExample) React to Explorer's \"Replace or Skip Files\":\n    extra_button_labels = Replace the file in the destination\n\nExample) Separate multiple entries with commas:\n    extra_button_labels = Replace the file in the destination, Install\n\nGeneric words such as \"Run\" or \"Next\" tend to match unintended screens.\nWizard buttons like \"Next\" are usually picked up by\nfollow_dialog_default_button without registering them here.\n\nMnemonics are removed automatically, so parentheses are not needed.\nWrite \"Save\" rather than \"&Save\" or \"Save(S)\".\n\nIf you are unsure of the exact label, set log_level = 2 and open the dialog;\nlog.txt will list the button names that were found.\n\nCaution: the cursor moves to buttons whose labels are listed here. Think\ntwice before registering irreversible actions such as Delete or Overwrite.\n(This application only moves the cursor; it never clicks.)"),

    // --- 追加分 ---
    ("lang.name", "English"),
    ("menu.ripple", "Show a target ring on arrival"),
    ("cfg.ripple_enabled", "Draw shrinking concentric rings where the cursor lands."),
    ("cfg.ripple_size", "Diameter of the rings in pixels (16-600)."),
    ("cfg.ripple_duration_ms", "How long the rings take to close in (0-3000 ms). 0 disables them."),
    ("cfg.ripple_color", "Ring colour as RRGGBB."),
    ("menu.theme", "Theme"),
    ("menu.theme.default", "Default (no theme)"),
    ("cfg.theme", "Theme name, matching a folder under the theme directory.
A theme may supply icon.ico, cursor_*.ani and sound.wav; anything
missing falls back to the built-in icon and the Windows cursor and sound.
Leave empty to use the defaults."),
];

fn fallback_of(key: &str) -> &'static str {
    FALLBACK
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, v)| *v)
        .unwrap_or("")
}

/// 言語ファイルが読み込み済みか。
pub fn is_loaded() -> bool {
    STRINGS.read().map(|s| s.is_some()).unwrap_or(false)
}

/// まだ読み込んでいなければ、指定の言語を読み込む。
///
/// `Config::load()` は設定ファイルが無いとき、その場で `save()` を呼んで
/// 既定値のファイルを作る。そのときコメントを言語ファイルから取れるよう、
/// 保存より前にこれを呼んでおく必要がある。
pub fn ensure_loaded(lang: &str) {
    if !is_loaded() {
        load(lang);
    }
}

/// メニューに出す文字列から、表示を壊す文字を取り除く。
///
/// 言語ファイルは利用者が編集できるため、改行やタブが混ざると
/// メニュー項目の表示が崩れる。長さも制限しておく。
fn sanitize_menu_text(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control())
        .take(64)
        .collect::<String>()
        .trim()
        .to_string()
}

/// 言語コードとして安全か。
///
/// この値はファイルパスの一部になるため、無検証で受け取ってはいけない。
/// `Path::join` は絶対パスを渡されると基準ディレクトリを丸ごと置き換えるので、
/// `C:\...` や `\\host\share` を指定されると `lang` の外を読めてしまう。
/// 英数字・ハイフン・アンダースコアだけを許可し、長さも制限する。
pub fn is_valid_code(code: &str) -> bool {
    !code.is_empty()
        && code.len() <= 32
        && code
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// 言語ファイルの上限。
///
/// 言語ファイルは第三者が作った翻訳を持ち込むことが想定されるため、
/// exe と同じ信頼レベルとは見なさず、読み込み時に正規化する。
const MAX_FILE_BYTES: u64 = 512 * 1024;
const MAX_KEY_LEN: usize = 128;
const MAX_VALUE_CHARS: usize = 2000;
const MAX_KEYS: usize = 2000;

/// 言語ファイルの値を、表示に使える形へ正規化する。
///
/// * NUL — Win32 に渡すと文字列がそこで切れるため取り除く
/// * その他の制御文字 — メニューの表示が崩れるため取り除く
/// * 改行 — `\n` からの変換ぶんだけを残す（config.ini のコメント用）
/// * 長さ — 画面に収まらない長さを防ぐため打ち切る
fn sanitize_value(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    // 残した文字数を自分で数える。out.chars().count() を毎回呼ぶと
    // 値の長さの 2 乗に比例し、長い値の多いファイルで読み込みが目に見えて遅くなる。
    let mut kept = 0usize;
    for c in raw.chars() {
        if kept >= MAX_VALUE_CHARS {
            break;
        }
        match c {
            '\n' => {
                out.push('\n');
                kept += 1;
            }
            c if c.is_control() => {}
            c => {
                out.push(c);
                kept += 1;
            }
        }
    }
    out
}

/// 言語ファイルを読み込む。存在しない場合は組み込みの文字列だけを使う。
pub fn load(lang: &str) {
    let mut map = HashMap::new();

    // 不正なコードは既定言語として扱う（パスには使わない）
    let lang = if is_valid_code(lang) {
        lang
    } else {
        DEFAULT_LANG
    };

    let path = exe_dir().join("lang").join(format!("{lang}.ini"));

    // 巨大なファイルを丸ごと読み込まない
    let size_ok = std::fs::metadata(&path)
        .map(|m| m.is_file() && m.len() <= MAX_FILE_BYTES)
        .unwrap_or(false);

    // ファイルが存在するのに使わなかった場合は理由を残す。
    // 黙って組み込みの英語に戻ると、「言語を選んだのに変わらない」という
    // 症状だけが残って原因が追えない。
    // 存在しない場合（＝既定の en を使うだけ）は普通のことなので何も言わない。
    if !size_ok && path.is_file() {
        crate::log::info(&format!(
            "言語ファイルを読み込めません（上限 {} KB を超えているか、読み取れません） path=\"{}\"",
            MAX_FILE_BYTES / 1024,
            path.display()
        ));
    }

    if size_ok {
        if let Ok(text) = std::fs::read_to_string(&path) {
            for line in text.trim_start_matches('\u{feff}').lines() {
                if map.len() >= MAX_KEYS {
                    break;
                }
                let line = line.trim();
                if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
                    continue;
                }
                let Some((k, v)) = line.split_once('=') else {
                    continue;
                };
                let key = k.trim();
                if key.is_empty() || key.len() > MAX_KEY_LEN {
                    continue;
                }
                // 値の中の \n は改行として扱う
                let value = sanitize_value(&v.trim().replace("\\n", "\n"));
                map.insert(key.to_string(), value);
            }
        }
    }

    if let Ok(mut slot) = STRINGS.write() {
        *slot = Some(map);
    }
}

/// キーに対応する文字列を返す。
pub fn t(key: &str) -> String {
    if let Ok(slot) = STRINGS.read() {
        if let Some(map) = slot.as_ref() {
            if let Some(v) = map.get(key) {
                if !v.is_empty() {
                    return v.clone();
                }
            }
        }
    }
    fallback_of(key).to_string()
}

/// `{0}` `{1}` ... を引数で置き換える。
pub fn tf(key: &str, args: &[&str]) -> String {
    let mut s = t(key);
    for (i, a) in args.iter().enumerate() {
        s = s.replace(&format!("{{{i}}}"), a);
    }
    s
}

/// 言語ファイルの先頭だけを読んで `lang.name` を取り出す。
///
/// 一覧はメニューを開くたびに作り直すため、表示名のためだけに
/// ファイル全体を読むのは無駄が大きい。
///
/// キー名は完全一致で見る。前方一致だと `lang.namespace` のような
/// 別のキーを表示名として拾ってしまう。
fn read_lang_name(path: &std::path::Path) -> Option<String> {
    use std::io::Read;

    /// 表示名は先頭付近にある想定。これを超える分は読まない
    const HEAD_BYTES: u64 = 8 * 1024;

    let mut head = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(HEAD_BYTES)
        .read_to_end(&mut head)
        .ok()?;

    // 途中で切れて壊れた文字は捨てる
    let text = String::from_utf8_lossy(&head);
    text.trim_start_matches('\u{feff}').lines().find_map(|l| {
        let (k, v) = l.split_once('=')?;
        if k.trim() == "lang.name" {
            Some(v.trim().to_string())
        } else {
            None
        }
    })
}

/// `lang` ディレクトリにある言語コードの一覧を返す。
///
/// 表示名は各ファイルの `lang.name` キーから取る。無ければコードをそのまま使う。
pub fn available() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let Ok(entries) = std::fs::read_dir(exe_dir().join("lang")) else {
        return out;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("ini") {
            continue;
        }
        let Some(code) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        // 読み込み時と同じ基準で弾く。ここを通さないと、
        // 選択できるのに読み込めないコードが一覧に出てしまう
        if !is_valid_code(code) {
            continue;
        }

        // 表示名を取るだけなので、ここでもサイズ上限をかける
        let size_ok = entry
            .metadata()
            .map(|m| m.is_file() && m.len() <= MAX_FILE_BYTES)
            .unwrap_or(false);
        if !size_ok {
            continue;
        }

        let name = read_lang_name(&path)
            .map(|s| sanitize_menu_text(&s))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| code.to_string());

        out.push((code.to_string(), name));

        // メニューに載る数を超えたら読むのをやめる。
        // theme::available() と揃えている。ここで打ち切らないと、
        // lang\ にファイルが大量にあるとき、メニューを開くたびに
        // 全ファイルを読むことになる
        if out.len() >= crate::tray::MAX_LANGUAGES {
            break;
        }
    }

    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_language_codes_are_accepted() {
        for code in ["en", "ja", "zh-TW", "pt_BR", "de"] {
            assert!(is_valid_code(code), "{code} が拒否された");
        }
    }

    /// 言語コードはパスの一部になるため、脱出を許してはならない。
    #[test]
    fn language_codes_cannot_escape_the_lang_directory() {
        for code in [
            "",
            "..",
            r"..\..\secret",
            "../../../etc/passwd",
            r"C:\Windows\System32\config\SAM",
            r"\\attacker\share\x",
            "ja/../../x",
            "ja;rm",
            &"a".repeat(33),
        ] {
            assert!(!is_valid_code(code), "{code} が受け入れられた");
        }
    }

    /// NUL は Win32 に渡すと文字列がそこで切れるため、必ず取り除く。
    #[test]
    fn control_characters_are_removed_from_values() {
        assert_eq!(sanitize_value("Exit\u{0}Hidden"), "ExitHidden");
        assert_eq!(sanitize_value("A\tB"), "AB");
        assert_eq!(sanitize_value("A\rB"), "AB");
    }

    /// `\n` から変換された改行だけは残す（config.ini のコメントに必要）。
    #[test]
    fn newlines_survive_sanitizing() {
        assert_eq!(sanitize_value("Line1\nLine2"), "Line1\nLine2");
    }

    #[test]
    fn overly_long_values_are_truncated() {
        let long = "あ".repeat(MAX_VALUE_CHARS + 500);
        assert_eq!(sanitize_value(&long).chars().count(), MAX_VALUE_CHARS);
    }

    #[test]
    fn menu_text_is_kept_on_one_line() {
        assert_eq!(sanitize_menu_text("  日本語\n(JP)  "), "日本語(JP)");
        assert!(sanitize_menu_text(&"x".repeat(200)).chars().count() <= 64);
    }

    /// 未定義のキーは組み込みの英語に落ちる。
    #[test]
    fn unknown_keys_fall_back_to_builtin_english() {
        assert_eq!(t("menu.exit"), fallback_of("menu.exit"));
        assert!(!t("menu.exit").is_empty());
    }

    /// 置換は指定した位置に正しく入る。
    #[test]
    fn placeholders_are_substituted_in_order() {
        assert_eq!(
            tf("msg.task.installed", &["MyTask"]),
            fallback_of("msg.task.installed").replace("{0}", "MyTask")
        );
    }
}

#[cfg(test)]
mod fallback_tests {
    use super::*;

    /// 組み込みフォールバックが `lang/en.ini` の全キーを持つことを確認する。
    ///
    /// 言語ファイルにキーを足したのに FALLBACK を更新し忘れると、
    /// 言語ファイルが読めない環境でメニュー項目が空欄になる。
    #[test]
    fn fallback_covers_every_key_in_en_ini() {
        let ini = include_str!("../lang/en.ini");
        let mut missing = Vec::new();
        for line in ini.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
                continue;
            }
            let Some((key, _)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim();
            if !FALLBACK.iter().any(|(k, _)| *k == key) {
                missing.push(key.to_string());
            }
        }
        assert!(
            missing.is_empty(),
            "FALLBACK に不足しているキー: {}",
            missing.join(", ")
        );
    }

    /// FALLBACK に `lang/en.ini` へ無いキーが残っていないことを確認する。
    ///
    /// 機能を削除したときに言語ファイルだけ直して FALLBACK を残すと、
    /// 使われない文字列が積み上がっていく。
    #[test]
    fn fallback_has_no_extra_keys() {
        let ini = include_str!("../lang/en.ini");
        let defined: Vec<&str> = ini
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty() && !l.starts_with(';') && !l.starts_with('#'))
            .filter_map(|l| l.split_once('='))
            .map(|(k, _)| k.trim())
            .collect();

        let extra: Vec<&str> = FALLBACK
            .iter()
            .map(|(k, _)| *k)
            .filter(|k| !defined.contains(k))
            .collect();

        assert!(
            extra.is_empty(),
            "FALLBACK に余分なキーがあります: {}",
            extra.join(", ")
        );
    }
}
