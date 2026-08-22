fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");

    // Windows 以外を対象にビルドされた場合は何もしない
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/icon.ico");
    res.set("FileDescription", "Dialog Cursor Mover");
    res.set("ProductName", "DialogCursorMover");
    // バージョンは Cargo.toml を唯一の出所にする
    if let Ok(version) = std::env::var("CARGO_PKG_VERSION") {
        res.set("FileVersion", &version);
        res.set("ProductVersion", &version);
    }
    res.set("LegalCopyright", "MIT License");

    // rc.exe / llvm-rc が見つからない環境でもビルド自体は通したい。
    // 埋め込みに失敗した場合は実行時に assets/icon.ico を直接読む経路にフォールバックする。
    if let Err(e) = res.compile() {
        println!("cargo:warning=アイコンリソースの埋め込みに失敗しました: {e}");
        println!(
            "cargo:warning=exe と同じフォルダに icon.ico を置けばトレイアイコンには反映されます"
        );
    }
}
