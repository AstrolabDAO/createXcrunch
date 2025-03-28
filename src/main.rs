use clap::Parser;
use crate::cli::{Cli, Commands, CliArgs};
use createxcrunch::{Config, RewardVariant, CreateVariant};
use alloy_primitives::hex;

mod cli;
mod webgpu;

// Helper function to create config from CLI args
fn factory_init_config(args: &CliArgs) -> Result<Config, String> {
    let reward = match (args.zeros, args.total, args.either, args.pattern.clone()) {
        (Some(zeros), None, false, None) => RewardVariant::LeadingZeros {
            zeros_threshold: zeros,
        },
        (None, Some(total), false, None) => RewardVariant::TotalZeros {
            zeros_threshold: total,
        },
        (Some(zeros), Some(total), false, None) => RewardVariant::LeadingAndTotalZeros {
            leading_zeros_threshold: zeros,
            total_zeros_threshold: total,
        },
        (Some(zeros), Some(total), true, None) => RewardVariant::LeadingOrTotalZeros {
            leading_zeros_threshold: zeros,
            total_zeros_threshold: total,
        },
        (None, None, false, Some(pattern)) => {
            let pattern = pattern
                .strip_prefix("0x")
                .unwrap_or(&pattern)
                .to_owned()
                .into_boxed_str();
            RewardVariant::Matching { pattern }
        }
        _ => unreachable!(),
    };

    Ok(Config::new(
        args.gpu_device_id,
        &args.factory,
        args.caller.as_deref(),
        args.chain_id,
        None,
        reward,
        &args.output,
    )?)
}

fn main() {
    let args = Cli::parse();

    match args.command {
        Commands::Create3(args) => {
            let config = match factory_init_config(&args) {
                Ok(config) => config,
                Err(e) => {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            };

            match webgpu::gpu(config) {
                Ok(_) => {},
                Err(e) => {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Commands::Create2(args) => {
            let init_code_hash_str = args.init_code_hash.clone();
            
            let mut config = match factory_init_config(&args.cli_args) {
                Ok(config) => config,
                Err(e) => {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            };
            
            // Decode the init code hash
            let init_code_hash_vec = hex::decode(&init_code_hash_str)
                .expect("could not decode init code hash argument");
            let init_code_hash = TryInto::<[u8; 32]>::try_into(init_code_hash_vec)
                .expect("invalid length for init code hash argument");
            
            // Set the create variant to Create2
            config.create_variant = CreateVariant::Create2 { init_code_hash };

            match webgpu::gpu(config) {
                Ok(_) => {},
                Err(e) => {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            }
        }
    }
} 