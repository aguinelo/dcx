fn main() {
    dcx_cli::run(std::iter::once("serve".to_owned()).chain(std::env::args().skip(1)));
}
