use ethrex_storage::Store;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let storage_path = "./geth-db-migrate/data/ethrex/storage";
    let store = Store::new(storage_path, ethrex_storage::EngineType::RocksDB)?;

    // Get the latest block number
    match store.get_latest_block_number().await {
        Ok(block_num) => {
            println!("Latest block number: {}", block_num);

            // Get the latest block to find state root
            if let Ok(Some(block)) = store.get_block_by_number(block_num).await {
                println!("State root: {}", block.header.state_root);

                // Try to check if this state root is valid
                match store.has_state_root(block.header.state_root) {
                    Ok(has_root) => {
                        println!("has_state_root({}) = {}", block.header.state_root, has_root);
                    }
                    Err(e) => {
                        println!("Error checking state root: {}", e);
                    }
                }

                // Try a few previous blocks too
                println!("\nChecking previous blocks:");
                for i in 1..=10 {
                    if block_num >= i {
                        if let Ok(Some(header)) = store.get_block_header(block_num - i) {
                            match store.has_state_root(header.state_root) {
                                Ok(has_root) => {
                                    println!(
                                        "  Block {}: state_root = {}, has_root = {}",
                                        header.number, header.state_root, has_root
                                    );
                                }
                                Err(e) => {
                                    println!("  Block {}: Error: {}", header.number, e);
                                }
                            }
                        }
                    }
                }
            }
        }
        Err(e) => {
            println!("Could not get latest block number: {}", e);
        }
    }

    Ok(())
}
