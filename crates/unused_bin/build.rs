fn main() {
    // Workaround because I think we're linking an older version of libunwind?
    // See https://github.com/libunwind/libunwind/issues/250
    // println!("cargo::rustc-link-arg=-lunwind-x86_64");
    // println!("cargo::rustc-link-arg=-llzma");
    // println!("cargo::rustc-link-arg=-lunwind");

    #[cfg(feature = "rstack")]
    println!("cargo::rustc-link-arg=-ldw");
}
