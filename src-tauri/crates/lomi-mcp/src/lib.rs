pub mod bounded_stdio;

mod server;

pub fn run(args: Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(server::serve(args))
}
