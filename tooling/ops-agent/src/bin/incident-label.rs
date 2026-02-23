use ethrex_ops_agent::storage::IncidentRepository;
use std::env;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        print_usage();
        return;
    }

    let db_path = env::var("OPS_AGENT_SQLITE_PATH").unwrap_or_else(|_| "ops-agent.sqlite".to_owned());
    let repository = match IncidentRepository::open(&db_path) {
        Ok(repository) => repository,
        Err(error) => {
            eprintln!("failed to open sqlite: {error}");
            return;
        }
    };

    match args[1].as_str() {
        "list" => {
            let limit = args[2].parse::<usize>().unwrap_or(20);
            match repository.list_recent(limit) {
                Ok(rows) => {
                    for row in rows {
                        let fp = match row.false_positive {
                            Some(true) => "false-positive",
                            Some(false) => "true-positive",
                            None => "unlabeled",
                        };
                        println!("#{} [{}|{}] {} ({})", row.id, row.scenario, row.severity, row.message, fp);
                    }
                }
                Err(error) => eprintln!("failed to list incidents: {error}"),
            }
        }
        "label" => {
            if args.len() < 4 {
                print_usage();
                return;
            }

            let id = match args[2].parse::<i64>() {
                Ok(id) => id,
                Err(error) => {
                    eprintln!("invalid incident id: {error}");
                    return;
                }
            };

            let is_false_positive = match args[3].as_str() {
                "fp" => true,
                "tp" => false,
                _ => {
                    eprintln!("label must be fp or tp");
                    return;
                }
            };

            if let Err(error) = repository.mark_false_positive(id, is_false_positive) {
                eprintln!("failed to update label: {error}");
                return;
            }

            match repository.false_positive_rate() {
                Ok(Some(rate)) => println!("updated #{id}. false_positive_rate={rate:.4}"),
                Ok(None) => println!("updated #{id}. false_positive_rate=N/A (no labeled incidents)"),
                Err(error) => eprintln!("failed to calculate false_positive_rate: {error}"),
            }
        }
        _ => print_usage(),
    }
}

fn print_usage() {
    eprintln!("usage:");
    eprintln!("  incident-label list <limit>");
    eprintln!("  incident-label label <incident_id> <fp|tp>");
}
