fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(lamp::out::shutdown(lamp::cli::main(&args)));
}
