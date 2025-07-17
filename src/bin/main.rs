// src/bin/main.rs

#[tokio::main]
async fn main() {
    println!("Distributed Block Builder - Simplified");
    println!("=====================================");
    println!("Architecture: Fixed role assignment at deployment");
    println!("- ~90% Simulation nodes");
    println!("- ~10% Merging nodes");
    println!("- Dynamic discovery of simulation→merger connections");
    println!("\nRun `cargo run --example simple_network` to see the protocol in action");
}
