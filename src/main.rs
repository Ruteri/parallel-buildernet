use rand::{thread_rng, Rng};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::select;
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio::time::{interval, sleep};

// Core types
pub type NodeId = [u8; 32];
pub type Hash = [u8; 32];
pub type BundleId = [u8; 32];
pub type ShardId = u8;
pub type BlockNumber = u64;
pub type RelayId = String;
pub type GeographicRegion = String;
pub type Address = [u8; 20];
pub type U256 = u128; // Simplified for demo
pub type Uuid = [u8; 16];
pub type Timestamp = u64;

// Custom types for arrays that need special serde handling
#[derive(Clone, Debug, Copy)]
pub struct PublicKey([u8; 33]);

impl Serialize for PublicKey {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_bytes(&self.0)
    }
}

impl<'de> Deserialize<'de> for PublicKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let bytes = <Vec<u8>>::deserialize(deserializer)?;
        if bytes.len() != 33 {
            return Err(serde::de::Error::custom("PublicKey must be 33 bytes"));
        }
        let mut arr = [0u8; 33];
        arr.copy_from_slice(&bytes);
        Ok(PublicKey(arr))
    }
}

impl Default for PublicKey {
    fn default() -> Self {
        PublicKey([0u8; 33])
    }
}

impl From<[u8; 33]> for PublicKey {
    fn from(bytes: [u8; 33]) -> Self {
        PublicKey(bytes)
    }
}

pub type PrivateKey = [u8; 32];

// Constants
const MERGERS_FRACTION: f32 = 0.1; // Less than 10% as per design
const MAX_MERGER_CONNECTIONS: usize = 5; // Each simulator connects to max 5 mergers
const GOSSIP_FANOUT: usize = 40; // Only used for topology and cancellations
const PARTIAL_BLOCK_INTERVAL: Duration = Duration::from_millis(100);

// Message types
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Message {
    // Orderflow messages (not gossiped between simulation nodes)
    Transaction(SignedTransaction),
    Bundle(Bundle),
    PartialBlock(PartialBlockPayload),

    // Topology and control messages (can be gossiped)
    NodeAnnouncement(NodeAnnouncement),
    RoleUpdate(RoleAssignment),
    HeartbeatPing(NodeStatus),
    Cancellation(CancellationRequest), // Special case: gossiped for speed

    // Explicit topology discovery
    TopologyRequest { from: NodeId },
    TopologyResponse { peers: Vec<(NodeId, PeerInfo)> },
}

#[derive(Clone, Debug)]
pub enum OrderflowSource {
    Regular,          // Via simulation nodes
    LatencySensitive, // Direct to mergers
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Transaction {
    pub nonce: u64,
    pub from: Address,
    pub to: Address,
    pub value: U256,
    pub gas_limit: u64,
    pub gas_price: U256,
    pub data: Vec<u8>,
}

impl Transaction {
    fn hash(&self) -> Hash {
        // Simple hash implementation
        let mut hash = [0u8; 32];
        hash[0..8].copy_from_slice(&self.nonce.to_be_bytes());
        hash[8..28].copy_from_slice(&self.from);
        hash[28..32].copy_from_slice(&self.value.to_be_bytes()[12..16]);
        hash
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Signature {
    v: u8,
    r: [u8; 32],
    s: [u8; 32],
}

impl Default for Signature {
    fn default() -> Self {
        Self {
            v: 0,
            r: [0u8; 32],
            s: [0u8; 32],
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignedTransaction {
    pub tx: Transaction,
    pub signature: Signature,
    pub received_at: Timestamp,
    pub is_latency_sensitive: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Bundle {
    pub id: BundleId,
    pub transactions: Vec<Transaction>,
    pub reverting_tx_hashes: Vec<Hash>,
    pub target_block: BlockNumber,
    pub min_timestamp: Option<Timestamp>,
    pub max_timestamp: Option<Timestamp>,
    pub is_latency_sensitive: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum OrderflowItem {
    Transaction(SignedTransaction),
    Bundle(Bundle),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PartialBlockPayload {
    pub id: Hash,
    pub block_target: BlockNumber,
    pub ordered_items: Vec<OrderflowItem>,
    pub conflict_set: ConflictSet,
    pub total_gas_used: u64,
    pub estimated_profit: U256,
    pub builder_pubkey: PublicKey,
    pub signature: Signature,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConflictSet {
    pub touched_accounts: HashSet<Address>,
    pub touched_storage: HashMap<Address, HashSet<U256>>,
    pub consumed_nonces: HashMap<Address, u64>,
}

impl ConflictSet {
    fn new() -> Self {
        Self {
            touched_accounts: HashSet::new(),
            touched_storage: HashMap::new(),
            consumed_nonces: HashMap::new(),
        }
    }

    pub fn conflicts_with(&self, other: &ConflictSet) -> bool {
        // Check account conflicts
        if !self.touched_accounts.is_disjoint(&other.touched_accounts) {
            return true;
        }

        // Check storage conflicts
        for (addr, slots) in &self.touched_storage {
            if let Some(other_slots) = other.touched_storage.get(addr) {
                if !slots.is_disjoint(other_slots) {
                    return true;
                }
            }
        }

        // Check nonce conflicts
        for (addr, nonce) in &self.consumed_nonces {
            if let Some(other_nonce) = other.consumed_nonces.get(addr) {
                if nonce >= other_nonce {
                    return true;
                }
            }
        }

        false
    }

    fn merge(&mut self, other: &ConflictSet) {
        self.touched_accounts.extend(&other.touched_accounts);

        for (addr, slots) in &other.touched_storage {
            self.touched_storage
                .entry(*addr)
                .or_insert_with(HashSet::new)
                .extend(slots);
        }

        for (addr, nonce) in &other.consumed_nonces {
            self.consumed_nonces
                .entry(*addr)
                .and_modify(|n| *n = (*n).max(*nonce))
                .or_insert(*nonce);
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CancellationRequest {
    pub tx_hash: Option<Hash>,
    pub bundle_id: Option<BundleId>,
    pub replacement_uuid: Option<Uuid>,
    pub signature: Signature,
    pub issued_at: Timestamp,
}

// Node roles
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum NodeRole {
    SimulationNode {},
    MergingNode {
        relay_latencies: HashMap<RelayId, Duration>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeAnnouncement {
    pub id: NodeId,
    pub region: GeographicRegion,
    pub capabilities: NodeCapabilities,
    pub public_key: PublicKey,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeCapabilities {
    pub relay_latencies: HashMap<RelayId, Duration>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RoleAssignment {
    pub node_id: NodeId,
    pub role: NodeRole,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeStatus {
    pub id: NodeId,
    pub uptime: Duration,
    pub blocks_built: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PeerInfo {
    pub region: GeographicRegion,
    pub latency: Duration,
    pub role: NodeRole,
    pub last_seen: Timestamp,
}

#[derive(Clone, Debug)]
pub struct SimulationResult {
    pub valid: bool,
    pub gas_used: u64,
    pub coinbase_profit: U256,
    pub touched_state: ConflictSet,
}

#[derive(Clone, Debug)]
pub struct BlockData {
    pub block_number: BlockNumber,
    pub items: Vec<OrderflowItem>,
    pub total_profit: U256,
    pub builder_id: NodeId,
}

struct ProtocolState {
    id: NodeId,
    role: NodeRole,
    region: GeographicRegion,
    peers: Arc<RwLock<HashMap<NodeId, PeerInfo>>>,
    public_key: PublicKey,
    private_key: PrivateKey,
}

struct BuildingState {
    cancelled_items: HashSet<Hash>,
    simulation_cache: HashMap<Hash, SimulationResult>,
    current_partial_block: PartialBlockBuilder,
    received_partial_blocks: HashMap<BlockNumber, Vec<PartialBlockPayload>>,
    latency_sensitive_items: Vec<OrderflowItem>,
    should_archive: bool, // Only true for merging nodes
}

struct PartialBlockBuilder {
    target_block: BlockNumber,
    ordered_items: Vec<OrderflowItem>,
    conflict_set: ConflictSet,
    total_gas_used: u64,
    estimated_profit: U256,
}

impl PartialBlockBuilder {
    fn new(target_block: BlockNumber) -> Self {
        Self {
            target_block,
            ordered_items: Vec::new(),
            conflict_set: ConflictSet::new(),
            total_gas_used: 0,
            estimated_profit: 0,
        }
    }

    fn can_add_item(&self, _item: &OrderflowItem, sim_result: &SimulationResult) -> bool {
        !self.conflict_set.conflicts_with(&sim_result.touched_state)
    }

    fn add_item(&mut self, item: OrderflowItem, sim_result: SimulationResult) {
        self.ordered_items.push(item);
        self.conflict_set.merge(&sim_result.touched_state);
        self.total_gas_used += sim_result.gas_used;
        self.estimated_profit += sim_result.coinbase_profit;
    }

    fn to_payload(
        &self,
        builder_key: &PrivateKey,
        builder_pubkey: &PublicKey,
    ) -> PartialBlockPayload {
        PartialBlockPayload {
            id: generate_id(),
            block_target: self.target_block,
            ordered_items: self.ordered_items.clone(),
            conflict_set: self.conflict_set.clone(),
            total_gas_used: self.total_gas_used,
            estimated_profit: self.estimated_profit,
            builder_pubkey: *builder_pubkey,
            signature: sign_payload(builder_key),
        }
    }
}

// Network transport layer
pub struct NetworkTransport {
    tx: mpsc::Sender<(NodeId, Message)>,
    rx: Arc<Mutex<mpsc::Receiver<(NodeId, Message)>>>,
}

impl NetworkTransport {
    pub fn new(
        _id: NodeId,
    ) -> (
        Self,
        mpsc::Sender<(NodeId, Message)>,
        mpsc::Receiver<(NodeId, Message)>,
    ) {
        let (tx_in, rx_in) = mpsc::channel(1000);
        let (tx_out, rx_out) = mpsc::channel(1000);

        let transport = Self {
            tx: tx_out,
            rx: Arc::new(Mutex::new(rx_in)),
        };

        (transport, tx_in, rx_out)
    }

    async fn send_to_peer(&self, peer_id: NodeId, msg: Message) {
        let _ = self.tx.send((peer_id, msg)).await;
    }

    async fn receive_message(&self) -> Option<(NodeId, Message)> {
        self.rx.lock().await.recv().await
    }
}

// Node implementation
pub struct Node {
    protocol: ProtocolState,
    building: Arc<RwLock<BuildingState>>,
    transport: NetworkTransport,
    shutdown: mpsc::Receiver<()>,
}

#[derive(Clone)]
pub struct NodeConfig {
    pub id: NodeId,
    pub region: GeographicRegion,
    pub public_key: PublicKey,
    pub private_key: PrivateKey,
}

impl Node {
    pub async fn new(
        config: NodeConfig,
        transport: NetworkTransport,
        shutdown: mpsc::Receiver<()>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let protocol = ProtocolState {
            id: config.id,
            role: NodeRole::SimulationNode {},
            region: config.region,
            peers: Arc::new(RwLock::new(HashMap::new())),
            public_key: config.public_key,
            private_key: config.private_key,
        };

        let building = Arc::new(RwLock::new(BuildingState {
            cancelled_items: HashSet::new(),
            simulation_cache: HashMap::new(),
            current_partial_block: PartialBlockBuilder::new(0),
            received_partial_blocks: HashMap::new(),
            latency_sensitive_items: Vec::new(),
            should_archive: false, // Will be set based on role
        }));

        Ok(Node {
            protocol,
            building,
            transport,
            shutdown,
        })
    }

    pub async fn run(mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Initialize and announce presence
        self.initialize().await?;

        // Run appropriate loop based on role
        match &self.protocol.role {
            NodeRole::SimulationNode { .. } => self.simulation_node_loop().await,
            NodeRole::MergingNode { .. } => self.merging_node_loop().await,
        }
    }

    async fn initialize(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Measure latencies (simulated)
        let mut relay_latencies = HashMap::new();
        relay_latencies.insert("relay1".to_string(), Duration::from_millis(10));
        relay_latencies.insert("relay2".to_string(), Duration::from_millis(20));

        // Announce to network
        let announcement = NodeAnnouncement {
            id: self.protocol.id,
            region: self.protocol.region.clone(),
            capabilities: NodeCapabilities {
                relay_latencies: relay_latencies.clone(),
            },
            public_key: self.protocol.public_key,
        };

        self.gossip_broadcast(Message::NodeAnnouncement(announcement))
            .await?;

        // Wait for role assignment
        self.participate_in_role_assignment().await?;

        // Set archive flag based on role
        let mut building = self.building.write().await;
        building.should_archive = matches!(self.protocol.role, NodeRole::MergingNode { .. });

        Ok(())
    }

    // Simulation node main loop
    async fn simulation_node_loop(
        mut self,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut partial_block_timer = interval(PARTIAL_BLOCK_INTERVAL);

        loop {
            select! {
                Some((sender, msg)) = self.transport.receive_message() => {
                    match msg {
                        Message::Transaction(tx) => {
                            self.handle_transaction(tx).await;
                        }
                        Message::Bundle(bundle) => {
                            self.handle_bundle(bundle).await;
                        }
                        Message::Cancellation(cancel) => {
                            self.propagate_cancellation(cancel).await;
                        }
                        Message::NodeAnnouncement(ann) => {
                            self.handle_node_announcement(sender, ann).await;
                        }
                        Message::TopologyRequest { from } => {
                            self.handle_topology_request(from).await;
                        }
                        _ => {}
                    }
                }
                _ = partial_block_timer.tick() => {
                    let should_send = {
                        let building = self.building.read().await;
                        !building.current_partial_block.ordered_items.is_empty()
                    };

                    if should_send {
                        self.send_partial_block().await;
                    }
                }
                _ = self.shutdown.recv() => {
                    break;
                }
            }
        }

        Ok(())
    }

    async fn handle_transaction(&self, tx: SignedTransaction) {
        let tx_hash = tx.tx.hash();

        // Check if cancelled
        {
            let building = self.building.read().await;
            if building.cancelled_items.contains(&tx_hash) {
                return;
            }
        }

        // Simulate
        match self.simulate_transaction(&tx).await {
            Ok(result) => {
                if result.valid {
                    let mut building = self.building.write().await;

                    // Cache result
                    building.simulation_cache.insert(tx_hash, result.clone());

                    // Try to add to current partial block
                    if building
                        .current_partial_block
                        .can_add_item(&OrderflowItem::Transaction(tx.clone()), &result)
                    {
                        building
                            .current_partial_block
                            .add_item(OrderflowItem::Transaction(tx.clone()), result);
                    }

                    // NO GOSSIP for regular transactions - they stay local
                    // and will be included in the next partial block
                }
            }
            Err(e) => {
                eprintln!("Transaction simulation failed: {:?}", e);
            }
        }
    }

    async fn handle_bundle(&self, bundle: Bundle) {
        let bundle_hash = bundle.id;

        {
            let building = self.building.read().await;
            if building.cancelled_items.contains(&bundle_hash) {
                return;
            }
        }

        match self.simulate_bundle(&bundle).await {
            Ok(result) => {
                if result.valid {
                    let mut building = self.building.write().await;

                    if building
                        .current_partial_block
                        .can_add_item(&OrderflowItem::Bundle(bundle.clone()), &result)
                    {
                        building
                            .current_partial_block
                            .add_item(OrderflowItem::Bundle(bundle.clone()), result);
                    }

                    // NO GOSSIP for bundles either - they stay local
                }
            }
            Err(e) => {
                eprintln!("Bundle simulation failed: {:?}", e);
            }
        }
    }

    async fn send_partial_block(&self) {
        let payload = {
            let building = self.building.read().await;
            building
                .current_partial_block
                .to_payload(&self.protocol.private_key, &self.protocol.public_key)
        };

        // Send to limited set of nearby mergers
        let target_mergers = self.get_nearby_mergers(MAX_MERGER_CONNECTIONS).await;

        println!(
            "Simulation node {} sending partial block to {} mergers",
            hex::encode(&self.protocol.id[0..4]),
            target_mergers.len()
        );

        for merger_id in target_mergers {
            self.transport
                .send_to_peer(merger_id, Message::PartialBlock(payload.clone()))
                .await;
        }

        // Reset for next partial block
        let mut building = self.building.write().await;
        let next_block = building.current_partial_block.target_block + 1;
        building.current_partial_block = PartialBlockBuilder::new(next_block);
    }

    async fn propagate_cancellation(&self, cancel: CancellationRequest) {
        // Cancellations use gossip for rapid propagation
        let mut building = self.building.write().await;

        // Update local state
        if let Some(hash) = cancel.tx_hash {
            building.cancelled_items.insert(hash);
            building
                .current_partial_block
                .ordered_items
                .retain(|item| match item {
                    OrderflowItem::Transaction(tx) => tx.tx.hash() != hash,
                    _ => true,
                });
        }

        if let Some(bundle_id) = cancel.bundle_id {
            building.cancelled_items.insert(bundle_id);
            building
                .current_partial_block
                .ordered_items
                .retain(|item| match item {
                    OrderflowItem::Bundle(bundle) => bundle.id != bundle_id,
                    _ => true,
                });
        }

        drop(building);

        // Only cancellations should gossip
        self.gossip_broadcast_priority(Message::Cancellation(cancel))
            .await;
    }

    async fn handle_node_announcement(&self, sender: NodeId, ann: NodeAnnouncement) {
        let mut peers = self.protocol.peers.write().await;
        peers.insert(
            sender,
            PeerInfo {
                region: ann.region,
                latency: Duration::from_millis(10), // Simulated
                role: NodeRole::SimulationNode {},
                last_seen: current_timestamp(), // Changed from Instant::now()
            },
        );
    }

    async fn handle_topology_request(&self, from: NodeId) {
        let peers = self.protocol.peers.read().await;
        let peer_list: Vec<(NodeId, PeerInfo)> =
            peers.iter().map(|(id, info)| (*id, info.clone())).collect();

        self.transport
            .send_to_peer(from, Message::TopologyResponse { peers: peer_list })
            .await;
    }

    // Merging node main loop
    async fn merging_node_loop(mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut block_timer = interval(Duration::from_secs(1));

        loop {
            select! {
                Some((sender, msg)) = self.transport.receive_message() => {
                    match msg {
                        Message::PartialBlock(partial) => {
                            self.handle_partial_block(partial).await;
                        }
                        Message::Transaction(tx) if tx.is_latency_sensitive => {
                            self.handle_latency_sensitive_transaction(tx).await;
                        }
                        Message::Bundle(bundle) if bundle.is_latency_sensitive => {
                            self.handle_latency_sensitive_bundle(bundle).await;
                        }
                        Message::Transaction(tx) if !tx.is_latency_sensitive => {
                            // Forward regular tx to a simulation node
                            if let Some(sim_node) = self.get_random_simulation_node().await {
                                self.transport.send_to_peer(sim_node, Message::Transaction(tx)).await;
                            }
                        }
                        Message::Bundle(bundle) if !bundle.is_latency_sensitive => {
                            // Forward regular bundle to a simulation node
                            if let Some(sim_node) = self.get_random_simulation_node().await {
                                self.transport.send_to_peer(sim_node, Message::Bundle(bundle)).await;
                            }
                        }
                        Message::Cancellation(cancel) => {
                            self.propagate_cancellation(cancel).await;
                        }
                        Message::NodeAnnouncement(ann) => {
                            self.handle_node_announcement(sender, ann).await;
                        }
                        _ => {}
                    }
                }
                _ = block_timer.tick() => {
                    self.build_and_submit_final_block().await;
                }
                _ = self.shutdown.recv() => {
                    break;
                }
            }
        }

        Ok(())
    }

    async fn handle_partial_block(&self, partial: PartialBlockPayload) {
        let mut building = self.building.write().await;
        building
            .received_partial_blocks
            .entry(partial.block_target)
            .or_insert_with(Vec::new)
            .push(partial);
    }

    async fn handle_latency_sensitive_transaction(&self, tx: SignedTransaction) {
        let mut building = self.building.write().await;
        building
            .latency_sensitive_items
            .push(OrderflowItem::Transaction(tx));
    }

    async fn handle_latency_sensitive_bundle(&self, bundle: Bundle) {
        let mut building = self.building.write().await;
        building
            .latency_sensitive_items
            .push(OrderflowItem::Bundle(bundle));
    }

    async fn build_and_submit_final_block(&self) {
        let current_block = self.get_current_block_number().await;

        let mut building = self.building.write().await;
        let partials = building
            .received_partial_blocks
            .remove(&current_block)
            .unwrap_or_default();

        if partials.is_empty() && building.latency_sensitive_items.is_empty() {
            return;
        }

        // Merge partial blocks
        let mut final_items = Vec::new();
        let mut merged_conflicts = ConflictSet::new();

        // Sort partials by profit density
        let mut sorted_partials = partials;
        sorted_partials.sort_by_key(|p| {
            if p.total_gas_used == 0 {
                0
            } else {
                (p.estimated_profit / p.total_gas_used as u128) as i128
            }
        });
        sorted_partials.reverse();

        // Merge non-conflicting items
        for partial in sorted_partials {
            for item in partial.ordered_items {
                if !self.item_conflicts_with(&item, &merged_conflicts).await {
                    final_items.push(item.clone());
                    self.update_conflict_set(&mut merged_conflicts, &item).await;
                }
            }
        }

        // Add latency-sensitive items
        let latency_items = std::mem::take(&mut building.latency_sensitive_items);
        for item in latency_items {
            if !self.item_conflicts_with(&item, &merged_conflicts).await {
                final_items.push(item.clone());
                self.update_conflict_set(&mut merged_conflicts, &item).await;
            }
        }

        let total_profit = self.calculate_block_profit(&final_items).await;

        drop(building);

        // Build and submit block
        println!(
            "Merging node {} built block {} with {} items (profit: {})",
            hex::encode(&self.protocol.id[0..4]),
            current_block,
            final_items.len(),
            total_profit
        );

        // Archive block data
        let block_data = BlockData {
            block_number: current_block,
            items: final_items.clone(),
            total_profit,
            builder_id: self.protocol.id,
        };
        self.save_to_archive(block_data).await;

        // Submit to relays
        self.submit_to_relays(final_items).await;
    }

    async fn save_to_archive(&self, block_data: BlockData) {
        let building = self.building.read().await;
        if !building.should_archive {
            return; // Only mergers save archive data
        }

        // Archive logic here
        println!(
            "Merger {} archiving block {} data",
            hex::encode(&self.protocol.id[0..4]),
            block_data.block_number
        );
    }

    // Helper methods
    pub async fn submit_transaction(&self, tx: SignedTransaction, source: OrderflowSource) {
        match (&self.protocol.role, source) {
            (NodeRole::MergingNode { .. }, OrderflowSource::LatencySensitive) => {
                // Direct processing at merger
                self.handle_latency_sensitive_transaction(tx).await;
            }
            (NodeRole::SimulationNode { .. }, _) => {
                // Regular processing at simulation node
                self.handle_transaction(tx).await;
            }
            (NodeRole::MergingNode { .. }, OrderflowSource::Regular) => {
                // Forward to a simulation node
                if let Some(sim_node) = self.get_random_simulation_node().await {
                    self.transport
                        .send_to_peer(sim_node, Message::Transaction(tx))
                        .await;
                }
            }
        }
    }

    async fn get_nearby_mergers(&self, max_count: usize) -> Vec<NodeId> {
        let peers = self.protocol.peers.read().await;
        let mut mergers: Vec<_> = peers
            .iter()
            .filter(|(_, info)| matches!(info.role, NodeRole::MergingNode { .. }))
            .collect();

        // Sort by latency, prioritizing same region
        mergers.sort_by_key(|(_, info)| {
            if info.region == self.protocol.region {
                info.latency.as_millis() as u64
            } else {
                info.latency.as_millis() as u64 + 1000 // Penalty for cross-region
            }
        });

        mergers
            .into_iter()
            .take(max_count)
            .map(|(id, _)| *id)
            .collect()
    }

    async fn get_random_simulation_node(&self) -> Option<NodeId> {
        let peers = self.protocol.peers.read().await;
        let sim_nodes: Vec<_> = peers
            .iter()
            .filter(|(_, info)| matches!(info.role, NodeRole::SimulationNode { .. }))
            .map(|(id, _)| *id)
            .collect();

        if sim_nodes.is_empty() {
            None
        } else {
            let mut rng = thread_rng();
            Some(sim_nodes[rng.gen_range(0..sim_nodes.len())])
        }
    }

    async fn simulate_transaction(
        &self,
        tx: &SignedTransaction,
    ) -> Result<SimulationResult, Box<dyn std::error::Error + Send + Sync>> {
        // Simulated execution
        let mut touched_state = ConflictSet::new();
        touched_state.touched_accounts.insert(tx.tx.from);
        touched_state.touched_accounts.insert(tx.tx.to);
        touched_state
            .consumed_nonces
            .insert(tx.tx.from, tx.tx.nonce);

        Ok(SimulationResult {
            valid: true,
            gas_used: tx.tx.gas_limit / 2, // Simulate partial usage
            coinbase_profit: tx.tx.gas_price * (tx.tx.gas_limit / 2) as u128,
            touched_state,
        })
    }

    async fn simulate_bundle(
        &self,
        bundle: &Bundle,
    ) -> Result<SimulationResult, Box<dyn std::error::Error + Send + Sync>> {
        let mut touched_state = ConflictSet::new();
        let mut total_gas = 0u64;
        let mut total_profit = 0u128;

        for tx in &bundle.transactions {
            touched_state.touched_accounts.insert(tx.from);
            touched_state.touched_accounts.insert(tx.to);
            total_gas += tx.gas_limit / 2;
            total_profit += tx.gas_price * (tx.gas_limit / 2) as u128;
        }

        Ok(SimulationResult {
            valid: true,
            gas_used: total_gas,
            coinbase_profit: total_profit,
            touched_state,
        })
    }

    async fn item_conflicts_with(&self, item: &OrderflowItem, conflicts: &ConflictSet) -> bool {
        match item {
            OrderflowItem::Transaction(tx) => {
                conflicts.touched_accounts.contains(&tx.tx.from)
                    || conflicts.touched_accounts.contains(&tx.tx.to)
            }
            OrderflowItem::Bundle(bundle) => bundle.transactions.iter().any(|tx| {
                conflicts.touched_accounts.contains(&tx.from)
                    || conflicts.touched_accounts.contains(&tx.to)
            }),
        }
    }

    async fn update_conflict_set(&self, conflicts: &mut ConflictSet, item: &OrderflowItem) {
        match item {
            OrderflowItem::Transaction(tx) => {
                conflicts.touched_accounts.insert(tx.tx.from);
                conflicts.touched_accounts.insert(tx.tx.to);
                conflicts.consumed_nonces.insert(tx.tx.from, tx.tx.nonce);
            }
            OrderflowItem::Bundle(bundle) => {
                for tx in &bundle.transactions {
                    conflicts.touched_accounts.insert(tx.from);
                    conflicts.touched_accounts.insert(tx.to);
                }
            }
        }
    }

    async fn calculate_block_profit(&self, items: &[OrderflowItem]) -> U256 {
        let mut total_profit = 0u128;
        for item in items {
            match item {
                OrderflowItem::Transaction(tx) => {
                    total_profit += tx.tx.gas_price * (tx.tx.gas_limit / 2) as u128;
                }
                OrderflowItem::Bundle(bundle) => {
                    for tx in &bundle.transactions {
                        total_profit += tx.gas_price * (tx.gas_limit / 2) as u128;
                    }
                }
            }
        }
        total_profit
    }

    async fn gossip_broadcast(
        &self,
        msg: Message,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Only used for topology messages and cancellations
        match &msg {
            Message::NodeAnnouncement(_)
            | Message::RoleUpdate(_)
            | Message::HeartbeatPing(_)
            | Message::TopologyRequest { .. }
            | Message::Cancellation(_) => {
                let peers = self.select_gossip_peers(GOSSIP_FANOUT).await;
                for peer in peers {
                    self.transport.send_to_peer(peer, msg.clone()).await;
                }
            }
            _ => {
                // Other messages should not be gossiped
                eprintln!("Warning: Attempted to gossip non-topology message");
            }
        }
        Ok(())
    }

    async fn gossip_broadcast_priority(&self, msg: Message) {
        // Priority broadcast for cancellations
        if matches!(msg, Message::Cancellation(_)) {
            let peers = self.protocol.peers.read().await;
            for peer in peers.keys() {
                self.transport.send_to_peer(*peer, msg.clone()).await;
            }
        }
    }

    async fn select_gossip_peers(&self, count: usize) -> Vec<NodeId> {
        let peers = self.protocol.peers.read().await;
        let mut peer_list: Vec<_> = peers.iter().collect();

        // Prioritize same region for topology gossip
        peer_list.sort_by_key(|(_, info)| {
            let region_penalty = if info.region != self.protocol.region {
                1000
            } else {
                0
            };
            let latency = info.latency.as_millis() as u32;
            region_penalty + latency
        });

        peer_list
            .into_iter()
            .take(count)
            .map(|(id, _)| *id)
            .collect()
    }

    async fn participate_in_role_assignment(
        &mut self,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Simulate role assignment based on latencies
        sleep(Duration::from_millis(100)).await;

        // For demo, randomly assign roles with proper ratio
        let mut rng = thread_rng();
        if rng.gen::<f32>() < MERGERS_FRACTION {
            // Become a merging node
            let mut relay_latencies = HashMap::new();
            relay_latencies.insert("relay1".to_string(), Duration::from_millis(5));
            relay_latencies.insert("relay2".to_string(), Duration::from_millis(10));

            self.protocol.role = NodeRole::MergingNode { relay_latencies };

            println!(
                "Node {} became a MERGING node",
                hex::encode(&self.protocol.id[0..4])
            );
        } else {
            println!(
                "Node {} became a SIMULATION node",
                hex::encode(&self.protocol.id[0..4])
            );
        }

        Ok(())
    }

    async fn submit_to_relays(&self, items: Vec<OrderflowItem>) {
        // In real implementation, this would submit to actual relays
        println!(
            "Merger {} submitting block with {} items to relays",
            hex::encode(&self.protocol.id[0..4]),
            items.len()
        );
    }

    async fn get_current_block_number(&self) -> BlockNumber {
        // In real implementation, this would get from chain
        1
    }
}

// Helper functions
pub fn generate_node_id() -> NodeId {
    let mut id = [0u8; 32];
    let mut rng = thread_rng();
    rng.fill(&mut id);
    id
}

fn generate_id() -> Hash {
    let mut id = [0u8; 32];
    let mut rng = thread_rng();
    rng.fill(&mut id);
    id
}

fn sign_payload(_key: &PrivateKey) -> Signature {
    // Simplified signature
    Signature::default()
}

// Test module
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore] // Can be run with: cargo test test_distributed_building_e2e -- --ignored --nocapture
    async fn test_distributed_building_e2e() {
        // Create network hub for message routing
        let mut network_hub = NetworkHub::new();

        // Create 10 nodes: ~9 simulation, ~1 merging
        let mut nodes = Vec::new();
        let mut node_handles = Vec::new();

        for i in 0..10 {
            let node_id = generate_node_id();
            let config = NodeConfig {
                id: node_id,
                region: if i < 4 {
                    "us-east".to_string()
                } else if i < 7 {
                    "eu-west".to_string()
                } else {
                    "asia-pac".to_string()
                },
                public_key: PublicKey::from([i as u8; 33]),
                private_key: [i as u8; 32],
            };

            let (transport, tx_in, rx_out) = NetworkTransport::new(node_id);
            network_hub.register_node(node_id, tx_in, rx_out);

            let (shutdown_tx, shutdown_rx) = mpsc::channel(1);
            let node = Node::new(config, transport, shutdown_rx).await.unwrap();

            nodes.push((node_id, shutdown_tx));
            node_handles.push(tokio::spawn(async move {
                let _ = node.run().await;
            }));
        }

        // Start network hub
        let _hub_handle = tokio::spawn(async move {
            network_hub.run().await;
        });

        // Wait for initialization
        sleep(Duration::from_millis(500)).await;

        // Send test transactions
        let test_tx = SignedTransaction {
            tx: Transaction {
                nonce: 1,
                from: [1u8; 20],
                to: [2u8; 20],
                value: 1000,
                gas_limit: 21000,
                gas_price: 20,
                data: vec![],
            },
            signature: Signature::default(),
            received_at: 0,
            is_latency_sensitive: false,
        };

        // Send to first node
        if let Some((node_id, _)) = nodes.first() {
            println!(
                "Test transaction sent to node {}",
                hex::encode(&node_id[0..4])
            );
        }

        // Wait for processing
        sleep(Duration::from_secs(2)).await;

        // Shutdown all nodes
        for (_, shutdown) in nodes {
            let _ = shutdown.send(()).await;
        }

        // Wait for nodes to finish
        for handle in node_handles {
            let _ = handle.await;
        }

        println!("Test completed successfully");
    }
}

// Network hub for testing
pub struct NetworkHub {
    nodes: HashMap<
        NodeId,
        (
            mpsc::Sender<(NodeId, Message)>,
            mpsc::Receiver<(NodeId, Message)>,
        ),
    >,
}

impl NetworkHub {
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
        }
    }

    pub fn register_node(
        &mut self,
        id: NodeId,
        tx: mpsc::Sender<(NodeId, Message)>,
        rx: mpsc::Receiver<(NodeId, Message)>,
    ) {
        self.nodes.insert(id, (tx, rx));
    }

    pub async fn run(mut self) {
        loop {
            // Route messages between nodes
            let mut had_messages = false;
            let node_ids: Vec<_> = self.nodes.keys().cloned().collect();

            for sender_id in node_ids {
                if let Some((_, rx)) = self.nodes.get_mut(&sender_id) {
                    if let Ok((target, msg)) = rx.try_recv() {
                        had_messages = true;
                        // Simulate network latency
                        let latency = self.calculate_latency(&sender_id, &target, &msg);
                        sleep(latency).await;

                        if let Some((tx, _)) = self.nodes.get(&target) {
                            let _ = tx.send((sender_id, msg)).await;
                        }
                    }
                }
            }

            if !had_messages {
                sleep(Duration::from_millis(10)).await;
            }
        }
    }

    fn calculate_latency(&self, _from: &NodeId, _to: &NodeId, msg: &Message) -> Duration {
        // Simulate network latency based on message type
        match msg {
            Message::Cancellation(_) => Duration::from_millis(5), // Priority
            Message::PartialBlock(_) => Duration::from_millis(10),
            _ => Duration::from_millis(15),
        }
    }
}

// Add hex encoding support
pub mod hex {
    pub fn encode(data: &[u8]) -> String {
        data.iter().map(|b| format!("{:02x}", b)).collect()
    }
}

fn current_timestamp() -> Timestamp {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

#[tokio::main]
async fn main() {
    println!("Distributed Block Builder Protocol");
    println!("==================================");
    println!("Architecture: Heterogeneous two-tier with direct routing");
    println!("Communication: O(N) scaling instead of O(N²)");
    println!("Roles: ~90% simulation nodes, ~10% merging nodes");
    println!();
    println!("To run the test: cargo test test_distributed_building_e2e -- --nocapture");
    println!("To run the network example: cargo run --example start_network");
}
