cargo clean;
cargo build --release --target aarch64-apple-darwin;
cargo build --release --target x86_64-pc-windows-gnu;

mkdir release;
zip -j release/neodepends-$1-aarch64-macos.zip target/aarch64-apple-darwin/release/neodepends artifacts/depends.jar;
zip -j release/neodepends-$1-x86_64-win.zip target/x86_64-pc-windows-gnu/release/neodepends.exe artifacts/depends.jar;