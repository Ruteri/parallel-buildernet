use distributed_block_builder::*;
use rand::{thread_rng, Rng};
use std::collections::HashMap;
use tokio::sync::mpsc;
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() {
    println!("Starting Distributed Block Building Network");
    println!("==========================================\n");

    let regions = vec!["us-east", "eu-west", "asia-pac"];
    let nodes_per_region = vec![4, 3, 3];

    let mut network_hub = NetworkHub::new();
    let mut nodes = Vec::new();
    let mut node_handles = Vec::new();

    // Start nodes in each region
    for (region_idx, (region, count)) in regions.iter().zip(nodes_per_region.iter()).enumerate() {
        println!("Starting {} nodes in {}", count, region);

        for i in 0..*count {
            let node_id = generate_node_id();
            let config = NodeConfig {
                id: node_id,
                region: region.to_string(),
                public_key: PublicKey::from([(region_idx * 10 + i) as u8; 33]),
                private_key: [(region_idx * 10 + i) as u8; 32],
            };

            let (transport, tx_in, rx_out) = NetworkTransport::new(node_id);
            network_hub.register_node(node_id, tx_in, rx_out);

            let (shutdown_tx, shutdown_rx) = mpsc::channel(1);
            let node = Node::new(config, transport, shutdown_rx).await.unwrap();

            nodes.push((node_id, region.to_string(), shutdown_tx));

            let node_id_copy = node_id;
            let region_copy = region.to_string();
            node_handles.push(tokio::spawn(async move {
                match node.run().await {
                    Ok(_) => println!(
                        "Node {} in {} shut down gracefully",
                        hex::encode(&node_id_copy[0..4]),
                        region_copy
                    ),
                    Err(e) => eprintln!(
                        "Node {} in {} error: {}",
                        hex::encode(&node_id_copy[0..4]),
                        region_copy,
                        e
                    ),
                }
            }));

            println!(
                "  Node {} started in {}",
                hex::encode(&node_id[0..4]),
                region
            );
        }
    }

    // Start network hub
    let hub_handle = tokio::spawn(async move {
        network_hub.run().await;
    });

    println!("\nAll nodes started. Network is initializing...");
    sleep(Duration::from_secs(2)).await;

    // Generate some orderflow
    println!("\nGenerating orderflow...");
    let mut rng = thread_rng();
    let mut tx_count = 0;
    let mut bundle_count = 0;

    // Run for 30 seconds
    let start = tokio::time::Instant::now();
    while start.elapsed() < Duration::from_secs(30) {
        // Generate transactions
        for _ in 0..5 {
            tx_count += 1;
            let tx = create_test_transaction(&mut rng, tx_count);

            // Pick a random simulation node
            let sim_nodes: Vec<_> = nodes
                .iter()
                .filter(|(_, _, _)| rng.gen_bool(0.8)) // 80% are simulation nodes
                .collect();

            if let Some((node_id, _, _)) = sim_nodes.get(rng.gen_range(0..sim_nodes.len().max(1))) {
                println!("TX {} → Node {}", tx_count, hex::encode(&node_id[0..4]));
                // In a real implementation, would send via network
            }
        }

        // Generate bundles
        if rng.gen_bool(0.3) {
            bundle_count += 1;
            let bundle = create_test_bundle(&mut rng, bundle_count);

            let sim_nodes: Vec<_> = nodes.iter().filter(|(_, _, _)| rng.gen_bool(0.8)).collect();

            if let Some((node_id, _, _)) = sim_nodes.get(rng.gen_range(0..sim_nodes.len().max(1))) {
                println!(
                    "Bundle {} → Node {}",
                    bundle_count,
                    hex::encode(&node_id[0..4])
                );
            }
        }

        // Generate latency-sensitive transactions
        if rng.gen_bool(0.1) {
            tx_count += 1;
            println!("Latency-sensitive TX {} sent", tx_count);
        }

        if tx_count % 10 == 0 {
            println!("Running for {} seconds...", start.elapsed().as_secs());
        }

        sleep(Duration::from_millis(500)).await;
    }

    println!("\nShutting down network...");

    // Shutdown all nodes
    for (node_id, region, shutdown) in nodes {
        println!(
            "Stopping node {} in {}",
            hex::encode(&node_id[0..4]),
            region
        );
        let _ = shutdown.send(()).await;
    }

    // Wait for nodes to finish
    for handle in node_handles {
        let _ = handle.await;
    }

    // Cancel hub
    hub_handle.abort();

    println!("\nNetwork shutdown complete.");
    println!("Total transactions: {}", tx_count);
    println!("Total bundles: {}", bundle_count);
}

fn create_test_transaction(rng: &mut impl Rng, nonce: u64) -> Transaction {
    Transaction {
        nonce,
        from: random_address(rng),
        to: random_address(rng),
        value: rng.gen_range(100..10000),
        gas_limit: 21000,
        gas_price: rng.gen_range(10..100),
        data: vec![],
    }
}

fn create_test_bundle(rng: &mut impl Rng, _id: u64) -> Bundle {
    let mut txs = Vec::new();
    for i in 0..rng.gen_range(2..5) {
        txs.push(Transaction {
            nonce: i,
            from: random_address(rng),
            to: random_address(rng),
            value: rng.gen_range(100..10000),
            gas_limit: 21000,
            gas_price: rng.gen_range(50..200),
            data: vec![],
        });
    }

    Bundle {
        id: random_hash(rng),
        transactions: txs,
        reverting_tx_hashes: vec![],
        target_block: 1,
        min_timestamp: None,
        max_timestamp: None,
        is_latency_sensitive: false,
    }
}

fn random_address(rng: &mut impl Rng) -> [u8; 20] {
    let mut addr = [0u8; 20];
    rng.fill(&mut addr);
    addr
}

fn random_hash(rng: &mut impl Rng) -> [u8; 32] {
    let mut hash = [0u8; 32];
    rng.fill(&mut hash);
    hash
}
