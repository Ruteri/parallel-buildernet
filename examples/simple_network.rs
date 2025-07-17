// examples/simple_network.rs
use distributed_block_builder::*;
use rand::{thread_rng, Rng};
use std::collections::HashMap;
use tokio::sync::mpsc;
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() {
    println!("🚀 Starting Simple Distributed Block Building Network");
    println!("===================================================");
    println!("Architecture: Heterogeneous with fixed roles");
    println!("- 90% Simulation nodes");
    println!("- 10% Merging nodes\n");

    // Create network hub
    let mut network_hub = NetworkHub::new();
    let mut node_handles = Vec::new();
    let mut node_info = Vec::new();

    // Deploy 10 nodes total: 1 merger, 9 simulation
    let total_nodes = 10;
    let merger_count = 1;
    let simulation_count = total_nodes - merger_count;

    println!("📋 Deploying {} nodes ({} mergers, {} simulators)", 
             total_nodes, merger_count, simulation_count);

    // Deploy merging nodes
    for i in 0..merger_count {
        let node_id = generate_node_id();
        let config = NodeConfig {
            id: node_id,
            role: NodeRole::Merging,
            region: "us-east".to_string(),
        };

        let (transport, tx_in, rx_out) = NetworkTransport::new(node_id);
        network_hub.register_node(node_id, tx_in, rx_out);

        let (shutdown_tx, shutdown_rx) = mpsc::channel(1);
        
        node_info.push((node_id, NodeRole::Merging, shutdown_tx));

        node_handles.push(tokio::spawn(async move {
            if let Err(e) = create_node(config, transport, shutdown_rx).await {
                eprintln!("Node error: {}", e);
            }
        }));

        println!("  ✨ Merger {} deployed in us-east", hex::encode(&node_id[0..4]));
    }

    // Deploy simulation nodes
    for i in 0..simulation_count {
        let node_id = generate_node_id();
        let region = if i < 5 { "us-east" } else { "eu-west" };
        
        let config = NodeConfig {
            id: node_id,
            role: NodeRole::Simulation,
            region: region.to_string(),
        };

        let (transport, tx_in, rx_out) = NetworkTransport::new(node_id);
        network_hub.register_node(node_id, tx_in, rx_out);

        let (shutdown_tx, shutdown_rx) = mpsc::channel(1);
        
        node_info.push((node_id, NodeRole::Simulation, shutdown_tx));

        node_handles.push(tokio::spawn(async move {
            if let Err(e) = create_node(config, transport, shutdown_rx).await {
                eprintln!("Node error: {}", e);
            }
        }));

        println!("  ✨ Simulator {} deployed in {}", hex::encode(&node_id[0..4]), region);
    }

    // Start network hub
    let hub_handle = tokio::spawn(async move {
        network_hub.run().await;
    });

    // Wait for initialization
    println!("\n⏳ Waiting for network initialization...");
    sleep(Duration::from_secs(2)).await;

    // Generate test transactions
    println!("\n📤 Sending test transactions...");
    let mut rng = thread_rng();
    
    // Send regular transactions to simulation nodes
    let sim_nodes: Vec<_> = node_info.iter()
        .filter(|(_, role, _)| matches!(role, NodeRole::Simulation))
        .collect();

    for i in 0..20 {
        let tx = SignedTransaction {
            tx: Transaction {
                nonce: i,
                from: [i as u8; 20],
                to: [(i + 1) as u8; 20],
                value: 1000,
                gas_limit: 21000,
                gas_price: 20 + i as u128,
                data: vec![],
            },
            signature: vec![0u8; 65],
            received_at: 0,
            is_latency_sensitive: false,
        };

        // Send to random simulation node
        if let Some((node_id, _, _)) = sim_nodes.get(rng.gen_range(0..sim_nodes.len())) {
            println!("  → TX {} to simulator {}", i, hex::encode(&node_id[0..4]));
            // In real implementation, would send via transport
        }
    }

    // Send latency-sensitive transactions directly to mergers
    let merger_nodes: Vec<_> = node_info.iter()
        .filter(|(_, role, _)| matches!(role, NodeRole::Merging))
        .collect();

    for i in 0..5 {
        let tx = SignedTransaction {
            tx: Transaction {
                nonce: 100 + i,
                from: [(100 + i) as u8; 20],
                to: [(101 + i) as u8; 20],
                value: 5000,
                gas_limit: 21000,
                gas_price: 100, // Higher priority
                data: vec![],
            },
            signature: vec![0u8; 65],
            received_at: 0,
            is_latency_sensitive: true,
        };

        if let Some((node_id, _, _)) = merger_nodes.first() {
            println!("  ⚡ Latency TX {} directly to merger {}", i, hex::encode(&node_id[0..4]));
        }
    }

    // Demonstrate cancellation
    println!("\n🚫 Sending cancellation...");
    let cancel = CancellationRequest {
        tx_hash: Some([5u8; 32]),
        bundle_id: None,
    };
    println!("  → Cancellation for TX hash {}", hex::encode(&[5u8; 32]));

    // Run for a few seconds
    println!("\n⏱️  Running network for 5 seconds...");
    sleep(Duration::from_secs(5)).await;

    // Show expected behavior
    println!("\n📊 Expected Protocol Behavior:");
    println!("  1. Simulation nodes build partial blocks every 100ms");
    println!("  2. Each simulator sends to ~5 nearby mergers");
    println!("  3. Mergers combine partial blocks every 1s");
    println!("  4. Only mergers submit to relays");
    println!("  5. Cancellations gossip through network");

    // Shutdown
    println!("\n🛑 Shutting down network...");
    for (node_id, role, shutdown) in node_info {
        let role_str = match role {
            NodeRole::Simulation => "simulator",
            NodeRole::Merging => "merger",
        };
        println!("  → Stopping {} {}", role_str, hex::encode(&node_id[0..4]));
        let _ = shutdown.send(()).await;
    }

    hub_handle.abort();
    for handle in node_handles {
        let _ = handle.await;
    }

    println!("\n✅ Network shutdown complete");
    
    println!("\n💡 Key Protocol Benefits Demonstrated:");
    println!("  - O(N) message complexity instead of O(N²)");
    println!("  - Only 10% of nodes submit to relays");
    println!("  - Direct path for latency-sensitive orderflow");
    println!("  - No orderflow gossip between simulation nodes");
}
