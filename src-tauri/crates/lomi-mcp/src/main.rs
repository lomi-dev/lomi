fn main() -> Result<(), Box<dyn std::error::Error>> {
    lomi_mcp::run(std::env::args().skip(1).collect())
}
