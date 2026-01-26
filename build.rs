fn main() {
    println!("cargo:rerun-if-changed=src/handler.c");
    
    cc::Build::new()
        .file("src/handler.c")
        .flag("-muintr")
        .flag("-O3")
        .flag("-std=gnu11")
        .compile("handler");
}
