fn main() {
    if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() != Ok("wasm32") {
        // Host builds (unit tests): link the tree-sitter grammar staticlib.
        // The dependency emits this directive too, but cargo has been
        // observed to drop it when linking cdylib test harnesses.
        println!("cargo:rustc-link-lib=static=parser-scanner");
        return;
    }
    // tree-sitter-typescript compiles parser.c + scanner.c into libparser-scanner.a.
    // lld for wasm32 cdylib won't pull it in automatically; --whole-archive forces it.
    println!("cargo:rustc-link-arg=--whole-archive");
    println!("cargo:rustc-link-arg=-lparser-scanner");
    println!("cargo:rustc-link-arg=--no-whole-archive");
}