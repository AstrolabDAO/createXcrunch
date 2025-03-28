use alloy_primitives::{hex, Address};
use fs4::FileExt;
use std::{
    fs::{File, OpenOptions},
};

pub enum CreateVariant {
    Create2 { init_code_hash: [u8; 32] },
    Create3,
}

pub enum RewardVariant {
    LeadingZeros {
        zeros_threshold: u8,
    },
    TotalZeros {
        zeros_threshold: u8,
    },
    LeadingAndTotalZeros {
        leading_zeros_threshold: u8,
        total_zeros_threshold: u8,
    },
    LeadingOrTotalZeros {
        leading_zeros_threshold: u8,
        total_zeros_threshold: u8,
    },
    Matching {
        pattern: Box<str>,
    },
}

pub enum SaltVariant {
    CrosschainSender {
        chain_id: [u8; 32],
        calling_address: [u8; 20],
    },
    Crosschain {
        chain_id: [u8; 32],
    },
    Sender {
        calling_address: [u8; 20],
    },
    Random,
}

pub struct Config<'a> {
    pub gpu_device: u8,
    pub factory_address: [u8; 20],
    pub salt_variant: SaltVariant,
    pub create_variant: CreateVariant,
    pub reward: RewardVariant,
    pub output: &'a str,
}

impl<'a> Config<'a> {
    pub fn new(
        gpu_device: u8,
        factory_address_str: &str,
        calling_address_str: Option<&str>,
        chain_id: Option<u64>,
        init_code_hash: Option<&str>,
        reward: RewardVariant,
        output: &'a str,
    ) -> Result<Self, &'static str> {
        // convert main arguments from hex string to vector of bytes
        let factory_address_vec =
            hex::decode(factory_address_str).expect("could not decode factory address argument");
        let calling_address_vec = calling_address_str.map(|calling_address| {
            hex::decode(calling_address).expect("could not decode calling address argument")
        });
        let init_code_hash_vec = init_code_hash.map(|init_code_hash| {
            hex::decode(init_code_hash).expect("could not decode init code hash argument")
        });

        // convert from vector to fixed array
        let factory_address = TryInto::<[u8; 20]>::try_into(factory_address_vec)
            .expect("invalid length for factory address argument");
        let calling_address = calling_address_vec.map(|calling_address_vec| {
            TryInto::<[u8; 20]>::try_into(calling_address_vec)
                .expect("invalid length for calling address argument")
        });
        let init_code_hash = init_code_hash_vec.map(|init_code_hash_vec| {
            TryInto::<[u8; 32]>::try_into(init_code_hash_vec)
                .expect("invalid length for init code hash argument")
        });
        let chain_id = chain_id.map(|chain_id| {
            let mut arr = [0u8; 32];
            arr[24..].copy_from_slice(&chain_id.to_be_bytes());
            arr
        });

        let create_variant = match init_code_hash {
            Some(init_code_hash) => CreateVariant::Create2 { init_code_hash },
            None => CreateVariant::Create3,
        };

        match &reward {
            RewardVariant::LeadingZeros { zeros_threshold }
            | RewardVariant::TotalZeros { zeros_threshold } => {
                validate_zeros_threshold(zeros_threshold)?;
            }
            RewardVariant::LeadingOrTotalZeros {
                leading_zeros_threshold,
                total_zeros_threshold,
            }
            | RewardVariant::LeadingAndTotalZeros {
                leading_zeros_threshold,
                total_zeros_threshold,
            } => {
                validate_zeros_threshold(leading_zeros_threshold)?;
                validate_zeros_threshold(total_zeros_threshold)?;
            }
            RewardVariant::Matching { pattern } => {
                validate_pattern(pattern)?;
            }
        }

        fn validate_zeros_threshold(threhsold: &u8) -> Result<(), &'static str> {
            if threhsold == &0u8 {
                return Err("threshold must be greater than 0");
            }
            if threhsold > &20u8 {
                return Err("threshold must be less than 20");
            }

            Ok(())
        }

        fn validate_pattern(pattern: &str) -> Result<(), &'static str> {
            if pattern.is_empty() {
                return Err("pattern cannot be empty");
            }

            // Check if pattern contains "..."
            if pattern.contains("...") {
                let parts: Vec<&str> = pattern.split("...").collect();
                if parts.len() != 2 {
                    return Err("pattern must have exactly one '...' separator");
                }

                let leading_part = parts[0];
                let trailing_part = parts[1];

                // Validate leading part
                if leading_part.is_empty() {
                    return Err("leading part of pattern cannot be empty");
                }
                if leading_part.len() % 2 != 0 {
                    return Err("leading part must have even number of characters");
                }
                if !leading_part.chars().all(|c| c.is_ascii_hexdigit()) {
                    return Err("leading part must contain only hex characters");
                }

                // We no longer require the leading part to be repeating
                // The leading part can be any valid hex string now

                // Validate trailing part if present
                if !trailing_part.is_empty() {
                    if trailing_part.len() != 2 {
                        return Err("trailing part must be exactly 2 characters");
                    }
                    if !trailing_part.chars().all(|c| c.is_ascii_hexdigit()) {
                        return Err("trailing part must contain only hex characters");
                    }
                }
            } else {
                // For simple patterns without "..."
                if pattern.len() != 2 {
                    return Err("simple pattern must be exactly 2 characters");
                }
                if !pattern.chars().all(|c| c.is_ascii_hexdigit()) {
                    return Err("pattern must contain only hex characters");
                }
            }

            Ok(())
        }

        let salt_variant = match (chain_id, calling_address) {
            (Some(chain_id), Some(calling_address)) if calling_address != [0u8; 20] => {
                SaltVariant::CrosschainSender {
                    chain_id,
                    calling_address,
                }
            }
            (Some(chain_id), None) => SaltVariant::Crosschain { chain_id },
            (None, Some(calling_address)) if calling_address != [0u8; 20] => {
                SaltVariant::Sender { calling_address }
            }
            _ => SaltVariant::Random,
        };

        if factory_address_str.chars().any(|c| c.is_uppercase()) {
            let factory_address_str = match factory_address_str.strip_prefix("0x") {
                Some(_) => factory_address_str.to_string(),
                None => format!("0x{}", factory_address_str),
            };
            match Address::parse_checksummed(factory_address_str, None) {
                Ok(_) => {}
                Err(_) => {
                    return Err("factory address uses invalid checksum");
                }
            }
        }

        if calling_address.is_some() {
            let calling_address_str = calling_address_str.unwrap();
            if calling_address_str.chars().any(|c| c.is_uppercase()) {
                let calling_address_str = match calling_address_str.strip_prefix("0x") {
                    Some(_) => calling_address_str.to_string(),
                    None => format!("0x{}", calling_address_str),
                };
                match Address::parse_checksummed(calling_address_str, None) {
                    Ok(_) => {}
                    Err(_) => {
                        return Err("caller address uses invalid checksum");
                    }
                }
            };
        };

        Ok(Self {
            gpu_device,
            factory_address,
            salt_variant,
            create_variant,
            reward,
            output,
        })
    }
}

pub fn output_file(config: &Config) -> File {
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .read(true)
        .append(true)
        .open(config.output)
        .expect("Failed to open output file");

    // exclusively lock the file for appending
    file.lock_exclusive().expect("Couldn't lock the file.");

    file
}
