use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

/// 最後に読み書きした時点の config.ini の更新時刻。
/// 外部から編集されたかどうかの判定に使う。
static LAST_SEEN_MTIME: Mutex<Option<SystemTime>> = Mutex::new(None);

fn current_mtime() -> Option<SystemTime> {
    fs::metadata(config_path()).ok()?.modified().ok()
}

fn remember_mtime() {
    if let Ok(mut slot) = LAST_SEEN_MTIME.lock() {
        *slot = current_mtime();
    }
}

/// 前回読み書きしたあとに、設定ファイルが外部から編集されたか。
///
/// トレイメニューの操作でファイル全体を書き戻すため、これを見ずに保存すると
/// 手で編集した内容をメモリ上の古い値で上書きしてしまう。
pub fn file_changed_externally() -> bool {
    let Ok(slot) = LAST_SEEN_MTIME.lock() else {
        return false;
    };
    match (*slot, current_mtime()) {
        (Some(known), Some(now)) => known != now,
        // 記録がない、あるいはファイルが消えた場合は判断できないので触らない
        _ => false,
    }
}

#[derive(Clone, Debug)]
pub struct Config {
    /// 表示言語（lang ディレクトリのファイル名）
    pub language: String,
    /// アイコン・カーソル・音を差し替えるテーマ名（空 = 既定）
    pub theme: String,
    /// 監視そのものの有効／無効
    pub enabled: bool,
    /// ダイアログ検出からカーソル移動までの遅延（ミリ秒）
    pub delay_ms: u32,
    /// 移動時に .wav を鳴らすか
    pub sound_enabled: bool,
    /// 再生する .wav のフルパス
    pub wav_path: String,
    /// 標準ダイアログ（ウィンドウクラス #32770）だけを対象にするか
    pub standard_dialog_only: bool,
    /// ダイアログが実際にフォアグラウンドになっている時だけ移動するか
    pub require_foreground: bool,
    /// 既にカーソルがボタン上にある場合は何もしない
    pub skip_if_cursor_inside: bool,
    /// 同じダイアログには一度だけ反応する
    pub move_once_per_dialog: bool,
    /// 自分自身のプロセスが出したダイアログを無視するか
    pub ignore_own_process: bool,
    /// カーソルを瞬間移動ではなく走らせる
    pub move_animation: bool,
    /// 走る時間（ミリ秒）
    pub move_duration_ms: u32,
    /// 走行時の左右の揺れ幅（px、0 で直線）
    pub move_wobble: u32,
    /// 走行中だけカーソル画像を差し替える（利用者が .ani を用意した場合のみ）
    pub cursor_animation: bool,
    /// 到達地点に同心円のアニメーションを表示する
    pub ripple_enabled: bool,
    /// 同心円の大きさ（直径 px）
    pub ripple_size: u32,
    /// 同心円が縮みきるまでの時間（ミリ秒）
    pub ripple_duration_ms: u32,
    /// 同心円の色（0xRRGGBB）
    pub ripple_color: u32,
    /// 走行中にユーザーがマウスを動かしたら中断する
    pub abort_on_user_move: bool,
    /// ユーザー操作とみなすカーソルのずれ幅（px）
    pub user_move_threshold: u32,
    /// タイトル変化も検出のきっかけにするか
    pub watch_title_changes: bool,
    /// フォーカス移動も検出のきっかけにするか
    pub watch_focus_changes: bool,
    /// ラベルが未登録でも、ダイアログの既定ボタンなら追従するか
    pub follow_dialog_default_button: bool,
    /// ブラウザでも既定ボタンへの追従を行うか
    pub follow_default_in_browser: bool,
    /// UI Automation を使って XAML / WinUI のダイアログにも対応するか
    pub uia_enabled: bool,
    /// UIA 走査を「ダイアログらしい形のウィンドウ」だけに限定するか
    pub uia_dialog_like_only: bool,
    /// UIA 走査で見る要素数の上限（多いほど確実だが遅くなる）
    pub uia_max_elements: u32,
    /// 診断ログのレベル (0=オフ / 1=通常 / 2=詳細)
    pub log_level: u32,
    /// ファイル/フォルダー選択ダイアログを無視するか
    pub skip_file_dialogs: bool,
    /// プログレスバーを持つ「処理中」ダイアログを無視するか
    pub skip_progress_dialogs: bool,
    /// これらの実行ファイルが出したダイアログを無視する
    pub exclude_processes: Vec<String>,
    /// タイトルにこれらの文字列を含むダイアログを無視する
    pub exclude_titles: Vec<String>,
    /// OK 相当とみなす追加のボタンラベル（カンマ区切り）
    pub extra_button_labels: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            language: crate::lang::DEFAULT_LANG.to_string(),
            theme: String::new(),
            enabled: true,
            delay_ms: 300,
            sound_enabled: true,
            wav_path: default_wav(),
            standard_dialog_only: true,
            require_foreground: true,
            skip_if_cursor_inside: true,
            move_once_per_dialog: true,
            ignore_own_process: false,
            move_animation: true,
            move_duration_ms: 320,
            move_wobble: 4,
            // 既定では Windows のカーソルをそのまま使う。
            // 差し替えるにはユーザーが .ani を用意する必要がある
            cursor_animation: false,
            ripple_enabled: true,
            ripple_size: 96,
            ripple_duration_ms: 420,
            ripple_color: 0x3A_84D6,
            abort_on_user_move: true,
            user_move_threshold: 12,
            watch_title_changes: true,
            watch_focus_changes: true,
            follow_dialog_default_button: true,
            follow_default_in_browser: false,
            uia_enabled: true,
            uia_dialog_like_only: true,
            uia_max_elements: 800,
            log_level: 0,
            skip_file_dialogs: true,
            skip_progress_dialogs: true,
            exclude_processes: Vec::new(),
            exclude_titles: Vec::new(),
            extra_button_labels: Vec::new(),
        }
    }
}

/// 実行ファイルのあるディレクトリ。
pub fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// 既定の再生音。
///
/// Windows のシステム音を指す。テーマに sound.wav があれば、
/// 再生時に main 側でそちらが優先される。
fn default_wav() -> String {
    r"C:\Windows\Media\Windows Ding.wav".to_string()
}

/// イベント処理のホットパス用に、`Config` からスカラー値だけを抜き出したもの。
///
/// `EVENT_OBJECT_NAMECHANGE` はデスクトップ全体で頻繁に飛ぶため、
/// そのたびに `Config` を clone すると `Vec<String>` などのヒープ確保が
/// 何度も走ってしまう。`Copy` なこちらを使えば確保はゼロになる。
#[derive(Clone, Copy)]
pub struct Flags {
    pub enabled: bool,
    pub delay_ms: u32,
    pub standard_dialog_only: bool,
    pub ignore_own_process: bool,
    pub uia_enabled: bool,
    pub uia_dialog_like_only: bool,
}

impl Config {
    pub fn flags(&self) -> Flags {
        Flags {
            enabled: self.enabled,
            delay_ms: self.delay_ms,
            standard_dialog_only: self.standard_dialog_only,
            ignore_own_process: self.ignore_own_process,
            uia_enabled: self.uia_enabled,
            uia_dialog_like_only: self.uia_dialog_like_only,
        }
    }
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.ini")
}

pub fn log_path() -> PathBuf {
    config_dir().join("log.txt")
}

/// 設定・ログの保存先。実行ファイルと同じディレクトリを使う。
/// 書き込めない場所（Program Files 等）に置かれている場合のみ %APPDATA% へ退避する。
pub fn config_dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                let probe = dir.join(".dcm_write_probe");
                if fs::write(&probe, b"").is_ok() {
                    let _ = fs::remove_file(&probe);
                    return dir.to_path_buf();
                }
            }
        }
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("DialogCursorMover")
    })
    .clone()
}

/// ASCII の大文字小文字を無視した部分一致。
///
/// ダイアログごとに 1 回しか呼ばれないため、素直に小文字化して比較する。
pub fn contains_ignore_ascii_case(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    haystack
        .to_ascii_lowercase()
        .contains(&needle.to_ascii_lowercase())
}

/// 読み込む config.ini の上限。
///
/// 言語ファイル（512 KB）やテーマ資産（8 MB）と同じ扱いにする。
/// 設定ディレクトリは書き込める場所なので、壊れた・肥大化したファイルが
/// 置かれうる。通常のファイルは 10 KB 前後。
const MAX_CONFIG_BYTES: u64 = 256 * 1024;

/// カンマ区切りリストの要素数の上限。
///
/// これらは 1 ダイアログにつき全件と突き合わせるため、
/// 際限なく増えると検出のたびに効いてくる。
const MAX_LIST_ITEMS: usize = 256;

/// カンマ区切りの値をリストにする。空の要素は捨てる。
fn parse_list(v: &str) -> Vec<String> {
    v.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .take(MAX_LIST_ITEMS)
        .collect()
}

fn parse_bool(v: &str, fallback: bool) -> bool {
    match v.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => true,
        "0" | "false" | "no" | "off" => false,
        _ => fallback,
    }
}

impl Config {
    /// INI を読み込む。存在しなければ既定値を書き出して返す。
    pub fn load() -> Config {
        let path = config_path();

        // 巨大なファイルを丸ごと読み込まない。
        // ここで既定値に落ちても、次回の保存で正常なファイルに書き換わる。
        if fs::metadata(&path)
            .map(|m| m.len() > MAX_CONFIG_BYTES)
            .unwrap_or(false)
        {
            crate::log::info(&format!(
                "設定ファイルが大きすぎるため既定値を使います（上限 {} KB） path=\"{}\"",
                MAX_CONFIG_BYTES / 1024,
                path.display()
            ));
            return Config::default();
        }

        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
            Err(_) => {
                // 初回起動。既定値でファイルを作る。
                // コメントは言語ファイルから取るので、保存より先に読み込ませる。
                let cfg = Config::default();
                crate::lang::ensure_loaded(&cfg.language);
                let _ = cfg.save();
                return cfg;
            }
        };

        let cfg = Config::parse_ini(&text);
        remember_mtime();
        cfg
    }

    /// INI 形式の文字列から設定を組み立てる。
    ///
    /// ファイル入出力から切り離してあるので、単体テストで往復を検証できる。
    pub fn parse_ini(text: &str) -> Config {
        let mut cfg = Config::default();

        for line in text.trim_start_matches('\u{feff}').lines() {
            let line = line.trim();
            if line.is_empty()
                || line.starts_with(';')
                || line.starts_with('#')
                || line.starts_with('[')
            {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let key = k.trim().to_ascii_lowercase();
            let v = v.trim();
            match key.as_str() {
                "theme" => {
                    let v = v.trim();
                    // パスの一部になるため書式を検証する。空は「既定」の意味
                    if v.is_empty() || crate::theme::is_valid_name(v) {
                        cfg.theme = v.to_string();
                    }
                }
                "language" => {
                    let v = v.trim();
                    // パスの一部になるため、書式を検証してから受け入れる
                    if crate::lang::is_valid_code(v) {
                        cfg.language = v.to_ascii_lowercase();
                    }
                }
                "enabled" => cfg.enabled = parse_bool(v, cfg.enabled),
                "delay_ms" => cfg.delay_ms = v.parse::<u32>().unwrap_or(cfg.delay_ms).min(60_000),
                "sound_enabled" => cfg.sound_enabled = parse_bool(v, cfg.sound_enabled),
                "wav_path" => cfg.wav_path = v.trim_matches('"').to_string(),
                "standard_dialog_only" => {
                    cfg.standard_dialog_only = parse_bool(v, cfg.standard_dialog_only)
                }
                "require_foreground" => {
                    cfg.require_foreground = parse_bool(v, cfg.require_foreground)
                }
                "skip_if_cursor_inside" => {
                    cfg.skip_if_cursor_inside = parse_bool(v, cfg.skip_if_cursor_inside)
                }
                "move_once_per_dialog" => {
                    cfg.move_once_per_dialog = parse_bool(v, cfg.move_once_per_dialog)
                }
                "ignore_own_process" => {
                    cfg.ignore_own_process = parse_bool(v, cfg.ignore_own_process)
                }
                "move_animation" => cfg.move_animation = parse_bool(v, cfg.move_animation),
                "move_duration_ms" => {
                    cfg.move_duration_ms = v
                        .parse::<u32>()
                        .unwrap_or(cfg.move_duration_ms)
                        .clamp(0, 5000)
                }
                "move_wobble" => {
                    cfg.move_wobble = v.parse::<u32>().unwrap_or(cfg.move_wobble).min(40)
                }
                "cursor_animation" => cfg.cursor_animation = parse_bool(v, cfg.cursor_animation),
                "ripple_enabled" => cfg.ripple_enabled = parse_bool(v, cfg.ripple_enabled),
                "ripple_size" => {
                    cfg.ripple_size = v.parse::<u32>().unwrap_or(cfg.ripple_size).clamp(16, 600)
                }
                "ripple_duration_ms" => {
                    cfg.ripple_duration_ms = v
                        .parse::<u32>()
                        .unwrap_or(cfg.ripple_duration_ms)
                        .clamp(0, 3000)
                }
                "ripple_color" => {
                    // 0xRRGGBB。先頭の # や 0x は取り除いてから解釈する
                    let t = v.trim_start_matches('#').trim_start_matches("0x");
                    cfg.ripple_color = u32::from_str_radix(t, 16)
                        .unwrap_or(cfg.ripple_color)
                        .min(0xFF_FFFF)
                }
                "abort_on_user_move" => {
                    cfg.abort_on_user_move = parse_bool(v, cfg.abort_on_user_move)
                }
                "user_move_threshold" => {
                    cfg.user_move_threshold = v
                        .parse::<u32>()
                        .unwrap_or(cfg.user_move_threshold)
                        .clamp(2, 500)
                }
                "watch_title_changes" => {
                    cfg.watch_title_changes = parse_bool(v, cfg.watch_title_changes)
                }
                "watch_focus_changes" => {
                    cfg.watch_focus_changes = parse_bool(v, cfg.watch_focus_changes)
                }
                "follow_dialog_default_button" => {
                    cfg.follow_dialog_default_button =
                        parse_bool(v, cfg.follow_dialog_default_button)
                }
                "follow_default_in_browser" => {
                    cfg.follow_default_in_browser = parse_bool(v, cfg.follow_default_in_browser)
                }
                "uia_enabled" => cfg.uia_enabled = parse_bool(v, cfg.uia_enabled),
                "uia_dialog_like_only" => {
                    cfg.uia_dialog_like_only = parse_bool(v, cfg.uia_dialog_like_only)
                }
                "uia_max_elements" => {
                    cfg.uia_max_elements = v
                        .parse::<u32>()
                        .unwrap_or(cfg.uia_max_elements)
                        .clamp(20, 5000)
                }
                "log_level" => cfg.log_level = v.parse::<u32>().unwrap_or(cfg.log_level).min(2),
                "skip_file_dialogs" => cfg.skip_file_dialogs = parse_bool(v, cfg.skip_file_dialogs),
                "skip_progress_dialogs" => {
                    cfg.skip_progress_dialogs = parse_bool(v, cfg.skip_progress_dialogs)
                }
                // 書いた文字列をそのまま保持する。比較時に大小文字を無視する
                "exclude_processes" => cfg.exclude_processes = parse_list(v),
                "exclude_titles" => cfg.exclude_titles = parse_list(v),
                "extra_button_labels" => cfg.extra_button_labels = parse_list(v),
                _ => {}
            }
        }
        cfg
    }

    /// 設定を INI として書き出す。
    ///
    /// コメントは言語ファイルの `cfg.<キー名>` から取る。
    /// 値の並びと書式は言語に依存しないので、言語を切り替えても
    /// 設定内容はそのまま引き継がれる。
    pub fn save(&self) -> std::io::Result<()> {
        let path = config_path();
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }

        let result = fs::write(&path, self.to_ini());
        remember_mtime();
        result
    }

    /// 実際に再生する wav のパス。
    ///
    /// テーマに sound.wav があればそれを優先する。ただし利用者が明示的に
    /// 別のファイルを選んでいる場合はその指定を尊重したいので、
    /// `wav_path` が既定値のままのときだけテーマを見る。
    ///
    /// 再生側とメニュー表示の双方から呼ぶ。片方だけで解決すると
    /// 「鳴っている音」と「現在の音として表示される名前」がずれる。
    pub fn effective_wav(&self) -> String {
        if self.wav_path == Config::default().wav_path {
            if let Some(p) = crate::theme::sound(&self.theme) {
                return p.to_string_lossy().into_owned();
            }
        }
        self.wav_path.clone()
    }

    /// 設定を INI 形式の文字列にする。
    ///
    /// ファイル出力から切り離してあるので、単体テストで往復を検証できる。
    pub fn to_ini(&self) -> String {
        let mut body = String::from("\u{feff}");
        let section = |title_key: &str, out: &mut String| {
            out.push_str("; ");
            out.push_str(&crate::lang::t(title_key).replace('\n', "\r\n; "));
            out.push_str("\r\n\r\n");
        };
        section("cfg.header", &mut body);

        // 各項目: コメント（複数行可）→ キー = 値
        let item = |key: &str, value: String, out: &mut String| {
            let comment = crate::lang::t(&format!("cfg.{key}"));
            for line in comment.lines() {
                out.push_str("; ");
                out.push_str(line);
                out.push_str("\r\n");
            }
            out.push_str(key);
            out.push_str(" = ");
            out.push_str(&value);
            out.push_str("\r\n\r\n");
        };

        item("language", self.language.clone(), &mut body);
        item("theme", self.theme.clone(), &mut body);
        item("enabled", self.enabled.to_string(), &mut body);
        item("delay_ms", self.delay_ms.to_string(), &mut body);
        item("sound_enabled", self.sound_enabled.to_string(), &mut body);
        item("wav_path", self.wav_path.clone(), &mut body);
        item(
            "standard_dialog_only",
            self.standard_dialog_only.to_string(),
            &mut body,
        );
        item(
            "require_foreground",
            self.require_foreground.to_string(),
            &mut body,
        );
        item(
            "skip_if_cursor_inside",
            self.skip_if_cursor_inside.to_string(),
            &mut body,
        );
        item(
            "move_once_per_dialog",
            self.move_once_per_dialog.to_string(),
            &mut body,
        );
        item(
            "ignore_own_process",
            self.ignore_own_process.to_string(),
            &mut body,
        );
        item("move_animation", self.move_animation.to_string(), &mut body);
        item(
            "move_duration_ms",
            self.move_duration_ms.to_string(),
            &mut body,
        );
        item("move_wobble", self.move_wobble.to_string(), &mut body);
        item(
            "cursor_animation",
            self.cursor_animation.to_string(),
            &mut body,
        );
        item("ripple_enabled", self.ripple_enabled.to_string(), &mut body);
        item("ripple_size", self.ripple_size.to_string(), &mut body);
        item(
            "ripple_duration_ms",
            self.ripple_duration_ms.to_string(),
            &mut body,
        );
        item(
            "ripple_color",
            format!("{:06X}", self.ripple_color),
            &mut body,
        );
        item(
            "abort_on_user_move",
            self.abort_on_user_move.to_string(),
            &mut body,
        );
        item(
            "user_move_threshold",
            self.user_move_threshold.to_string(),
            &mut body,
        );
        item(
            "watch_title_changes",
            self.watch_title_changes.to_string(),
            &mut body,
        );
        item(
            "watch_focus_changes",
            self.watch_focus_changes.to_string(),
            &mut body,
        );
        item(
            "follow_dialog_default_button",
            self.follow_dialog_default_button.to_string(),
            &mut body,
        );
        item(
            "follow_default_in_browser",
            self.follow_default_in_browser.to_string(),
            &mut body,
        );
        item("uia_enabled", self.uia_enabled.to_string(), &mut body);
        item(
            "uia_dialog_like_only",
            self.uia_dialog_like_only.to_string(),
            &mut body,
        );
        item(
            "uia_max_elements",
            self.uia_max_elements.to_string(),
            &mut body,
        );
        item("log_level", self.log_level.to_string(), &mut body);
        item(
            "skip_file_dialogs",
            self.skip_file_dialogs.to_string(),
            &mut body,
        );
        item(
            "skip_progress_dialogs",
            self.skip_progress_dialogs.to_string(),
            &mut body,
        );
        item(
            "exclude_processes",
            self.exclude_processes.join(", "),
            &mut body,
        );
        item("exclude_titles", self.exclude_titles.join(", "), &mut body);
        item(
            "extra_button_labels",
            self.extra_button_labels.join(", "),
            &mut body,
        );

        body
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 全項目が INI へ書き出され、読み戻せることを確認する。
    ///
    /// 設定項目を追加したときに、読み込みか書き出しの片方を実装し忘れると
    /// このテストが落ちる。手作業での確認に頼らないための保険。
    #[test]
    fn round_trip_preserves_every_field() {
        let original = Config {
            language: "ja".to_string(),
            theme: "cat".to_string(),
            enabled: false,
            delay_ms: 1234,
            sound_enabled: false,
            wav_path: r"C:\sounds\my sound.wav".to_string(),
            move_animation: false,
            move_duration_ms: 999,
            move_wobble: 13,
            cursor_animation: false,
            ripple_enabled: false,
            ripple_size: 123,
            ripple_duration_ms: 456,
            ripple_color: 0xAB_CDEF,
            abort_on_user_move: false,
            user_move_threshold: 42,
            standard_dialog_only: false,
            require_foreground: false,
            skip_if_cursor_inside: false,
            move_once_per_dialog: false,
            ignore_own_process: true,
            watch_title_changes: false,
            watch_focus_changes: false,
            follow_dialog_default_button: false,
            follow_default_in_browser: true,
            uia_enabled: false,
            uia_dialog_like_only: false,
            uia_max_elements: 321,
            skip_file_dialogs: false,
            skip_progress_dialogs: false,
            exclude_processes: vec!["a.exe".into(), "b.exe".into()],
            exclude_titles: vec!["Title One".into(), "Title Two".into()],
            extra_button_labels: vec!["実行".into(), "Install".into()],
            log_level: 2,
        };

        let restored = Config::parse_ini(&original.to_ini());

        assert_eq!(restored.language, original.language);
        assert_eq!(restored.theme, original.theme);
        assert_eq!(restored.enabled, original.enabled);
        assert_eq!(restored.delay_ms, original.delay_ms);
        assert_eq!(restored.sound_enabled, original.sound_enabled);
        assert_eq!(restored.wav_path, original.wav_path);
        assert_eq!(restored.move_animation, original.move_animation);
        assert_eq!(restored.move_duration_ms, original.move_duration_ms);
        assert_eq!(restored.move_wobble, original.move_wobble);
        assert_eq!(restored.cursor_animation, original.cursor_animation);
        assert_eq!(restored.ripple_enabled, original.ripple_enabled);
        assert_eq!(restored.ripple_size, original.ripple_size);
        assert_eq!(restored.ripple_duration_ms, original.ripple_duration_ms);
        assert_eq!(restored.ripple_color, original.ripple_color);
        assert_eq!(restored.abort_on_user_move, original.abort_on_user_move);
        assert_eq!(restored.user_move_threshold, original.user_move_threshold);
        assert_eq!(restored.standard_dialog_only, original.standard_dialog_only);
        assert_eq!(restored.require_foreground, original.require_foreground);
        assert_eq!(
            restored.skip_if_cursor_inside,
            original.skip_if_cursor_inside
        );
        assert_eq!(restored.move_once_per_dialog, original.move_once_per_dialog);
        assert_eq!(restored.ignore_own_process, original.ignore_own_process);
        assert_eq!(restored.watch_title_changes, original.watch_title_changes);
        assert_eq!(restored.watch_focus_changes, original.watch_focus_changes);
        assert_eq!(
            restored.follow_dialog_default_button,
            original.follow_dialog_default_button
        );
        assert_eq!(
            restored.follow_default_in_browser,
            original.follow_default_in_browser
        );
        assert_eq!(restored.uia_enabled, original.uia_enabled);
        assert_eq!(restored.uia_dialog_like_only, original.uia_dialog_like_only);
        assert_eq!(restored.uia_max_elements, original.uia_max_elements);
        assert_eq!(restored.skip_file_dialogs, original.skip_file_dialogs);
        assert_eq!(
            restored.skip_progress_dialogs,
            original.skip_progress_dialogs
        );
        assert_eq!(restored.exclude_processes, original.exclude_processes);
        assert_eq!(restored.exclude_titles, original.exclude_titles);
        assert_eq!(restored.extra_button_labels, original.extra_button_labels);
        assert_eq!(restored.log_level, original.log_level);
    }

    /// 書き出した INI に、構造体の全フィールドが現れることを確認する。
    ///
    /// 上の往復テストは代入を書き忘れると素通りしてしまうので、
    /// キーの存在自体も確かめておく。
    #[test]
    fn every_key_is_written() {
        let ini = Config::default().to_ini();
        for key in [
            "language",
            "theme",
            "enabled",
            "delay_ms",
            "sound_enabled",
            "wav_path",
            "move_animation",
            "move_duration_ms",
            "move_wobble",
            "cursor_animation",
            "ripple_enabled",
            "ripple_size",
            "ripple_duration_ms",
            "ripple_color",
            "abort_on_user_move",
            "user_move_threshold",
            "standard_dialog_only",
            "require_foreground",
            "skip_if_cursor_inside",
            "move_once_per_dialog",
            "ignore_own_process",
            "watch_title_changes",
            "watch_focus_changes",
            "follow_dialog_default_button",
            "follow_default_in_browser",
            "uia_enabled",
            "uia_dialog_like_only",
            "uia_max_elements",
            "skip_file_dialogs",
            "skip_progress_dialogs",
            "exclude_processes",
            "exclude_titles",
            "extra_button_labels",
            "log_level",
        ] {
            assert!(
                ini.lines().any(|l| l.starts_with(&format!("{key} ="))),
                "{key} が config.ini に書き出されていません"
            );
        }
    }

    /// 不正な値を書いても既定値に落ちるだけで、壊れないことを確認する。
    #[test]
    fn invalid_values_fall_back_to_defaults() {
        let d = Config::default();
        let cfg = Config::parse_ini(
            "delay_ms = not a number\n\
             log_level = 99\n\
             move_wobble = -5\n\
             uia_max_elements = 1\n\
             enabled = maybe\n\
             = orphan value\n\
             no equals sign here\n",
        );
        assert_eq!(cfg.delay_ms, d.delay_ms, "数値でない値は既定値に戻る");
        assert_eq!(cfg.log_level, 2, "範囲外は上限に丸められる");
        assert_eq!(cfg.move_wobble, d.move_wobble, "負数は既定値に戻る");
        assert_eq!(cfg.uia_max_elements, 20, "下限に丸められる");
        assert_eq!(cfg.enabled, d.enabled, "真偽値でない値は既定値に戻る");
    }

    /// 言語コードは絶対パスや親ディレクトリ指定を受け付けない。
    #[test]
    fn language_rejects_path_traversal() {
        for evil in [
            r"C:\Windows\System32\config\SAM",
            r"..\..\secret",
            "../../../etc/passwd",
            r"\\attacker\share\x",
        ] {
            let cfg = Config::parse_ini(&format!("language = {evil}\n"));
            assert_eq!(
                cfg.language,
                Config::default().language,
                "不正な言語コードが受け入れられました: {evil}"
            );
        }
    }

    /// 除外指定は大文字小文字を区別しない。
    #[test]
    fn exclusion_matching_ignores_case() {
        assert!(contains_ignore_ascii_case("FastCopy ver5.11", "fastcopy"));
        assert!(contains_ignore_ascii_case("fastcopy ver5.11", "FastCopy"));
        assert!(!contains_ignore_ascii_case("FastCopy", ""));
        assert!(!contains_ignore_ascii_case("", "x"));
    }

    /// 書いた文字列がそのまま保存される（比較時だけ小文字化する）。
    #[test]
    fn exclusion_values_keep_their_original_case() {
        let cfg = Config::parse_ini("exclude_titles = FastCopy, GIMP\n");
        assert_eq!(cfg.exclude_titles, vec!["FastCopy", "GIMP"]);
    }

    /// カンマ区切りの値は際限なく増やせない。
    ///
    /// これらは 1 ダイアログにつき全件と突き合わせるため、無制限だと
    /// 設定ファイルを置き換えるだけで検出のたびに重くできてしまう。
    /// 言語ファイルやテーマ資産には上限があるのに、ここだけ無かった。
    #[test]
    fn comma_separated_lists_are_capped() {
        let many = (0..MAX_LIST_ITEMS + 50)
            .map(|i| format!("t{i}"))
            .collect::<Vec<_>>()
            .join(",");
        let cfg = Config::parse_ini(&format!("exclude_titles = {many}\n"));
        assert_eq!(cfg.exclude_titles.len(), MAX_LIST_ITEMS);
        assert_eq!(cfg.exclude_titles[0], "t0", "先頭から順に採用される");
    }

    #[test]
    fn bool_parsing_accepts_common_spellings() {
        for v in ["true", "TRUE", "1", "yes", "on"] {
            assert!(parse_bool(v, false), "{v} が true にならない");
        }
        for v in ["false", "FALSE", "0", "no", "off"] {
            assert!(!parse_bool(v, true), "{v} が false にならない");
        }
        assert!(parse_bool("???", true), "不明な値は既定値のまま");
    }
}
