use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use wgpu::util::DeviceExt;
use hex;

use createxcrunch::{Config, RewardVariant};
use alloy_primitives::Address;

// Number of bytes in the salt/nonce
const SALT_SIZE: usize = 32;
// Number of bytes in an Ethereum address
const ADDRESS_SIZE: usize = 20;
// Output size from Keccak256 hash (same as in the shader)
const KECCAK256_OUTPUT_SIZE: u32 = 8; // 8 u32 values (32 bytes)

// Store the raw pattern for dynamic matching
static mut RAW_PATTERN: Option<String> = None;

// Aligned buffer for solutions
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct SolutionBuffer {
    // Salt that produced the matching address
    salt: [u32; 8], // 32 bytes as u32 array
    // The resulting address (only need 20 bytes but use 32 for alignment)
    address: [u32; 8], // 32 bytes as u32 array
}

pub fn gpu(config: Config) -> Result<(), Box<dyn std::error::Error>> {
    println!(
        "Setting up WebGPU miner using Metal backend..."
    );

    // Store the pattern string for dynamic matching
    if let RewardVariant::Matching { pattern } = &config.reward {
        // Store the pattern globally for use in check_eth_address_pattern
        unsafe {
            RAW_PATTERN = Some(pattern.to_string());
        }
    }

    // Clear output file first to remove old non-matching results
    {
        println!("Clearing previous output file at: {}", config.output);
        let mut file = std::fs::File::create(&config.output)?;
        
        // Display the actual pattern we're looking for
        if let RewardVariant::Matching { pattern } = &config.reward {
            writeln!(file, "# Mining for addresses matching pattern {}", pattern)?;
        } else {
            writeln!(file, "# Mining for addresses matching pattern")?;
        }
        
        writeln!(file, "# Format: salt -> ethereum_address")?;
        writeln!(file, "# Started: {}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs())?;
        writeln!(file)?;
    }

    // Track found addresses
    let mut found: u64 = 0;
    let found_list: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(vec![]));
    
    // Set up a controller for terminal output
    let term = console::Term::stdout();
    
    // Start time tracking
    let start_time = Instant::now();
    let mut last_status_time = Instant::now();
    let mut total_cycles: u64 = 0;
    
    // Initialize WebGPU
    let instance = wgpu::Instance::default();
    
    // Find an adapter (GPU)
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    })).ok_or("Failed to find a suitable GPU adapter")?;
    
    println!("Using GPU: {}", adapter.get_info().name);
    
    // Create device and queue
    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("CreateXCrunch Device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            memory_hints: wgpu::MemoryHints::default(),
        },
        None,
    ))?;
    
    // Prepare shader
    let shader_src = include_str!("kernels/keccak256.wgsl");
    
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Keccak Shader"),
        source: wgpu::ShaderSource::Wgsl(shader_src.into()),
    });
    
    // Determine optimal work size and workgroup size based on GPU capabilities
    let workgroup_size = 256; // This should match the workgroup_size in the shader
    
    // Reduce work size to stay within buffer limits (max 268,435,456 bytes)
    // Max buffer size is 268,435,456 bytes. Each hash is 32 bytes, so max entries = 8,388,608
    // Use a much smaller value to be safe
    let work_size: u32 = 1_000_000; // 1M nonces
    let workgroups = (work_size + workgroup_size - 1) / workgroup_size;
    
    println!("Using work batch size of {} nonces per iteration", work_size);
    println!("Using {} workgroups of {} threads each", workgroups, workgroup_size);
    
    // Create buffers
    // 1. Input buffer for messages (salt data)
    let message_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Message Buffer"),
        contents: bytemuck::cast_slice(&[0u32; 8]), // 32 bytes for salt
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
    });
    
    // 2. Number of messages
    let num_messages_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Num Messages Buffer"),
        contents: bytemuck::cast_slice(&[work_size as u32]),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
    });
    
    // 3. Message sizes
    let message_sizes_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Message Sizes Buffer"),
        contents: bytemuck::cast_slice(&[8u32, 8u32]), // Input length and stride
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
    });
    
    // 4. Output buffer for hashes
    let hash_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Hash Buffer"),
        // Each result is (2 u32 for nonce + 8 u32 for hash) * 4 bytes per u32
        size: (work_size as u64) * ((2 + KECCAK256_OUTPUT_SIZE as u64) * 4),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    
    // 5. Log buffer
    let log_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Log Buffer"),
        size: 64 * 4, // 64 u32 values
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    
    // 6. Solutions buffer (for staging on CPU)
    let solution_staging_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Solution Staging Buffer"),
        size: std::mem::size_of::<SolutionBuffer>() as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    
    // Create a new CPU-side validation buffer
    let validation_buffer_size = work_size as usize * (2 + KECCAK256_OUTPUT_SIZE as usize) * 4; // Size in bytes
    let cpu_validation_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("CPU Validation Buffer"),
        size: validation_buffer_size as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    
    // Parse pattern from the user-provided pattern string
    let (pattern_value, pattern_flags, pattern_length) = match &config.reward {
        RewardVariant::Matching { pattern } => {
            parse_pattern(pattern)
        },
        // For backward compatibility, convert old reward types
        RewardVariant::LeadingZeros { zeros_threshold } => {
            // Convert leading zeros to a single value
            (0u32, 4u32, (*zeros_threshold) as u32) // Flag 4 for legacy leading zeros
        },
        RewardVariant::TotalZeros { zeros_threshold } => {
            // Convert total zeros to a single value
            (0u32, 5u32, (*zeros_threshold) as u32) // Flag 5 for legacy total zeros
        },
        RewardVariant::LeadingAndTotalZeros { leading_zeros_threshold, total_zeros_threshold } => {
            // Store leading zeros in pattern_value, total zeros in flags higher bits
            ((*leading_zeros_threshold) as u32, 6u32, (*total_zeros_threshold) as u32)
        },
        RewardVariant::LeadingOrTotalZeros { leading_zeros_threshold, total_zeros_threshold } => {
            // Just use leading zeros for simplicity
            ((*leading_zeros_threshold) as u32, 7u32, (*total_zeros_threshold) as u32)
        },
    };

    // Set up the pattern configuration buffer
    let pattern_config_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Pattern Config Buffer"),
        contents: bytemuck::cast_slice(&[
            pattern_value,    // Main pattern value (prefix byte or zero count)
            pattern_flags,    // Pattern type flags (1=leading, 2=trailing, 3=both)
            pattern_length,   // Length of pattern to match
            0u32,             // Reserved for future use
        ]),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
    });
    
    // Create bind group layout
    let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Bind Group Layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 4,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 5,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    });
    
    // Create bind group
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Bind Group"),
        layout: &bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: message_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: num_messages_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: message_sizes_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: hash_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: log_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: pattern_config_buffer.as_entire_binding(),
            },
        ],
    });
    
    // Create pipeline layout
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Pipeline Layout"),
        bind_group_layouts: &[&bind_group_layout],
        push_constant_ranges: &[],
    });
    
    // Create compute pipeline
    let compute_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("Compute Pipeline"),
        layout: Some(&pipeline_layout),
        module: &shader,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    
    // Main loop
    let mut nonce: u64 = 0;
    let should_exit = false;
    
    println!("Mining started. Press Ctrl+C to exit.");
    
    'outer: loop {
        // Update input buffer with new nonce
        queue.write_buffer(&message_buffer, 0, bytemuck::cast_slice(&[nonce as u32]));
        
        // Create command encoder
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Command Encoder"),
        });
        
        // Compute pass to find matching addresses
        {
            let mut compute_pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Compute Pass"),
                timestamp_writes: None,
            });
            compute_pass.set_pipeline(&compute_pipeline);
            compute_pass.set_bind_group(0, &bind_group, &[]);
            compute_pass.dispatch_workgroups(workgroups, 1, 1);
        }
        
        // Copy all hash results to CPU validation buffer
        encoder.copy_buffer_to_buffer(
            &hash_buffer,
            0,
            &cpu_validation_buffer,
            0,
            validation_buffer_size as u64,
        );
        
        // Submit command buffer
        queue.submit(std::iter::once(encoder.finish()));
        
        // Map validation buffer for reading
        let buffer_slice = cpu_validation_buffer.slice(..);
        let (sender, receiver) = futures_intrusive::channel::shared::oneshot_channel();
        buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
            sender.send(result).unwrap();
        });
        
        // Wait for GPU mapping
        device.poll(wgpu::Maintain::Wait);
        
        if let Some(Ok(())) = pollster::block_on(receiver.receive()) {
            let data = buffer_slice.get_mapped_range();
            let results = bytemuck::cast_slice::<u8, u32>(&data);
            
            // Pre-allocate a vector to store matches for batch processing
            let mut matches = Vec::with_capacity(16); // Usually we won't find more than a few matches
            
            // Process each result (nonce + hash)
            for i in 0..work_size as usize {
                let base_idx = i * (2 + KECCAK256_OUTPUT_SIZE as usize);
                
                // Extract nonce
                let nonce_low = results[base_idx];
                let nonce_high = results[base_idx + 1];
                
                // Extract hash
                let mut hash = [0u32; KECCAK256_OUTPUT_SIZE as usize];
                for j in 0..KECCAK256_OUTPUT_SIZE as usize {
                    hash[j] = results[base_idx + 2 + j];
                }
                
                // Convert hash to bytes (in big-endian order for Ethereum compatibility)
                let mut hash_bytes = [0u8; 32];
                for j in 0..8 {
                    // Convert to big-endian format to match Ethereum convention
                    let bytes = hash[j].to_be_bytes();
                    hash_bytes[j*4..(j+1)*4].copy_from_slice(&bytes);
                }
                
                // Get the Ethereum address part (bytes 12-31)
                let eth_address_start = 12;
                let eth_address_bytes = &hash_bytes[eth_address_start..32];
                
                // Check if the address matches our pattern
                let matches_pattern = check_eth_address_pattern(eth_address_bytes, pattern_value as u8, pattern_flags, pattern_length);
                
                if matches_pattern {
                    // Add to the matches collection for batch processing
                    matches.push((nonce_low, nonce_high, hash_bytes));
                }
            }
            
            // Batch process matches
            if !matches.is_empty() {
                found += matches.len() as u64;
                
                // Open the file once outside the loop for better performance
                let mut file = std::fs::OpenOptions::new()
                    .create(true)
                    .write(true)
                    .append(true)
                    .open(&config.output)
                    .unwrap();
                    
                // Process all matches
                for (nonce_low, nonce_high, hash_bytes) in matches {
                    // Create salt bytes
                    let mut salt_bytes = [0u8; 32];
                    salt_bytes[0..4].copy_from_slice(&nonce_low.to_le_bytes());
                    salt_bytes[4..8].copy_from_slice(&nonce_high.to_le_bytes());
                    
                    // Format the salt as hex
                    let salt_hex = format!("0x{}", hex::encode(salt_bytes));
                    
                    // Get the Ethereum address part (bytes 12-31)
                    let eth_address_start = 12;
                    let eth_address_bytes = &hash_bytes[eth_address_start..32];
                    
                    // Create checksummed address
                    let address = Address::from_slice(eth_address_bytes);
                    let checksummed_address = format!("{}", address.to_checksum(None));
                    
                    // Write to output file
                    if let Err(e) = writeln!(file, "{} -> {}", salt_hex, checksummed_address) {
                        eprintln!("Error writing to output file: {}", e);
                    }
                    
                    // Add to found list
                    let mut found_list_guard = found_list.lock().unwrap();
                    found_list_guard.push(format!("{} -> {}", salt_hex, checksummed_address));
                }
            }
            
            // Unmap buffer
            drop(data);
            cpu_validation_buffer.unmap();
        }
        
        // Increment nonce for next iteration
        nonce += work_size as u64;
        total_cycles += 1;
        
        // Update status every second
        if last_status_time.elapsed() >= Duration::from_secs(1) {
            let elapsed = start_time.elapsed();
            let rate = nonce as f64 / elapsed.as_secs_f64() / 1_000_000.0;
            
            term.clear_line().unwrap();
            print!(
                "total runtime: {:02}:{:02}:{:02.2} ({} cycles)                      work size per cycle: {}",
                elapsed.as_secs() / 3600,
                (elapsed.as_secs() % 3600) / 60,
                (elapsed.as_secs() % 60) as f64 + (elapsed.subsec_nanos() as f64 / 1_000_000_000.0),
                total_cycles,
                work_size
            );
            print!("\nrate: {:.2} million attempts per second                  total found this run: {}", rate, found);
            
            // Print additional pattern information based on config
            match &config.reward {
                RewardVariant::Matching { pattern } => {
                    print!("\ncurrent search space: {}            threshold: mining for Create3 address matching pattern 0x{}", nonce, pattern);
                }
                RewardVariant::LeadingZeros { zeros_threshold } => {
                    print!("\ncurrent search space: {}            threshold: mining for Create3 address with {} leading zero bytes", nonce, zeros_threshold);
                }
                RewardVariant::TotalZeros { zeros_threshold } => {
                    print!("\ncurrent search space: {}            threshold: mining for Create3 address with {} total zero bytes", nonce, zeros_threshold);
                }
                _ => {}
            }
            
            last_status_time = Instant::now();
        }
        
        // Check if the user wants to stop (non-blocking)
        if should_exit {
            println!("\nUser interrupted.");
            break 'outer;
        }
    }
    
    Ok(())
}

fn parse_pattern(pattern: &str) -> (u32, u32, u32) {
    let mut pattern_value = 0u32;
    let mut pattern_flags = 0u32;
    let mut pattern_length = 0u32;
    let mut trailing_value = 0u32;

    // Check for empty pattern
    if pattern.is_empty() {
        return (0, 0, 0);
    }

    println!("Raw pattern: '{}'", pattern);

    // Check if pattern contains "..."
    if pattern.contains("...") {
        let parts: Vec<&str> = pattern.split("...").collect();
        
        if parts.len() == 2 {
            let leading_part = parts[0];
            let trailing_part = parts[1];
            
            // Check for special non-repeating patterns like "ABCD...EF"
            if !leading_part.is_empty() && leading_part.len() % 2 == 0 && 
               !trailing_part.is_empty() && trailing_part.len() == 2 {
                // Store special pattern flag for non-repeating patterns
                pattern_flags = 99; // Special flag for non-repeating patterns
                
                // For non-repeating patterns, we'll use the original pattern in the check function
                // Just store some placeholder values
                println!("Detected non-repeating pattern: {}...{}", leading_part, trailing_part);
                
                // Return special values that will be checked in the check_eth_address_pattern function
                return (99, 99, 0);
            }
            
            // Check if leading part is a repeating pattern (e.g., "BBBBBB")
            if !leading_part.is_empty() && leading_part.len() % 2 == 0 {
                let first_byte = &leading_part[0..2];
                let mut is_repeating = true;
                
                // Check if all bytes in leading part are the same
                for i in (0..leading_part.len()).step_by(2) {
                    if &leading_part[i..i+2] != first_byte {
                        is_repeating = false;
                        break;
                    }
                }
                
                if is_repeating {
                    // Parse the repeating byte value (e.g., "BB")
                    if let Ok(value) = u8::from_str_radix(first_byte, 16) {
                        pattern_value = value as u32;
                        pattern_length = (leading_part.len() / 2) as u32;
                        
                        // Check if we have a trailing pattern
                        if !trailing_part.is_empty() && trailing_part.len() == 2 {
                            if let Ok(trailing) = u8::from_str_radix(trailing_part, 16) {
                                trailing_value = trailing as u32;
                                pattern_flags = 5; // New flag: multiple leading + specific trailing
                                println!("Detected complex pattern: {} leading 0x{:02X} bytes + trailing 0x{:02X}", 
                                       pattern_length, pattern_value, trailing_value);
                            }
                        } else {
                            pattern_flags = 4; // Just multiple leading bytes
                            println!("Detected multiple leading pattern: {} bytes of 0x{:02X}", 
                                   pattern_length, pattern_value);
                        }
                    }
                }
            }
        }
    }
    
    // Store trailing value in upper bits of pattern_value if we have one
    if pattern_flags == 5 {
        pattern_value = pattern_value | (trailing_value << 8);
    }
    
    println!("Final config: value=0x{:04X}, flags={}, length={}", 
             pattern_value, pattern_flags, pattern_length);
    return (pattern_value, pattern_flags, pattern_length);
}

// Add this debug method to print how Ethereum addresses are derived
fn debug_ethereum_address(salt_bytes: &[u8; 32], hash_output: &[u8; 32]) {
    let eth_address_start = 12;
    println!("Salt: {}", hex::encode(salt_bytes));
    println!("Hash result: {}", hex::encode(hash_output));
    println!("Derived ETH address: 0x{}", hex::encode(&hash_output[eth_address_start..32]));
    println!("First byte of ETH address: 0x{:02X}", hash_output[eth_address_start]);
}

// Fix the process_output function to be standalone
fn process_output(output_buffer: &[u8], log_buffer: &[u8]) -> usize {
    let mut matches_found = 0;

    // Extract the salt (first 8 bytes of log_buffer)
    let mut salt_bytes = [0u8; 32];
    let mut hash_output = [0u8; 32];

    // Extract the first 8 bytes into salt_bytes (rest remains zero)
    for i in 0..8 {
        salt_bytes[i] = log_buffer[i];
    }

    // Extract hash output (next 32 bytes)
    for i in 0..32 {
        hash_output[i] = log_buffer[i + 8];
    }

    // Convert to hex string for display
    let salt_hex = hex::encode(salt_bytes);
    
    // Get the Ethereum address from the hash (last 20 bytes of keccak hash)
    let eth_address_start = 12;
    let eth_address = &hash_output[eth_address_start..32];
    let eth_address_hex = hex::encode(eth_address);
    
    // Check if the address starts with BB (we're looking for 0xBB...)
    if eth_address.len() >= 2 && eth_address[0] == 0xBB {
        println!("MATCH FOUND! Address: 0x{}", eth_address_hex);
        matches_found += 1;
        
        // Write result to output file
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("output.txt") {
            
            if let Err(e) = writeln!(file, "0x{} -> 0x{}", salt_hex, eth_address_hex) {
                eprintln!("Error writing to output file: {}", e);
            }
        }
    } else {
        // For debugging: print all found addresses
        println!("Found address: 0x{} (from salt: 0x{})", eth_address_hex, salt_hex);
    }

    matches_found
}

// Optimize the check_eth_address_pattern function for better performance
fn check_eth_address_pattern(address_bytes: &[u8], pattern_value: u8, pattern_flags: u32, pattern_length: u32) -> bool {
    // Fast path for common case - empty address
    if address_bytes.is_empty() {
        return false;
    }
    
    // Special case for non-repeating patterns (pattern_flags = 99)
    if pattern_flags == 99 {
        // Use cached pattern parts rather than parsing the pattern every time
        static mut PREFIX: Option<String> = None;
        static mut SUFFIX: Option<String> = None;
        
        unsafe {
            // Initialize pattern parts if needed
            if PREFIX.is_none() || SUFFIX.is_none() {
                if let Some(pattern_str) = &RAW_PATTERN {
                    if pattern_str.contains("...") {
                        let parts: Vec<&str> = pattern_str.split("...").collect();
                        if parts.len() == 2 {
                            PREFIX = Some(parts[0].to_lowercase());
                            SUFFIX = Some(parts[1].to_lowercase());
                        }
                    }
                }
            }
            
            // Get cached pattern parts
            if let (Some(prefix), Some(suffix)) = (&PREFIX, &SUFFIX) {
                // Compute address hex only once
                let address_hex = hex::encode(address_bytes);
                
                // Fast check - if not starting with prefix or ending with suffix, return early
                if !address_hex.starts_with(prefix) || !address_hex.ends_with(suffix) {
                    return false;
                }
                
                // Both prefix and suffix match
                return true;
            }
        }
        
        // Fallback - use full pattern
        let address_hex = hex::encode(address_bytes);
        
        // Fix the temporary value issue by creating a longer-lived empty string
        let empty_str = String::new();
        let pattern_str = unsafe { 
            match &RAW_PATTERN {
                Some(p) => p,
                None => &empty_str,
            }
        };
        
        if pattern_str.contains("...") {
            let parts: Vec<&str> = pattern_str.split("...").collect();
            if parts.len() == 2 {
                let prefix = parts[0].to_lowercase();
                let suffix = parts[1].to_lowercase();
                
                return address_hex.starts_with(&prefix) && address_hex.ends_with(&suffix);
            }
        }
        
        return address_hex.starts_with(&pattern_str.to_lowercase());
    }
    
    // For other pattern types, use optimized byte-level checks instead of string conversions
    match pattern_flags {
        0 | 1 => {
            // Simple pattern - just match the first byte
            // This is extremely fast - just a single byte comparison
            return address_bytes[0] == pattern_value;
        },
        2 => {
            // Trailing pattern - check last byte
            return address_bytes[address_bytes.len() - 1] == pattern_value;
        },
        3 => {
            // Both leading and trailing - check first and last bytes are the same
            return address_bytes[0] == pattern_value && 
                   address_bytes[address_bytes.len() - 1] == pattern_value;
        },
        4 => {
            // Multiple leading bytes - check the first N bytes
            let num_bytes = pattern_length as usize;
            
            // Fast path - often we check just 1-2 bytes
            if num_bytes == 1 {
                return address_bytes[0] == pattern_value;
            } else if num_bytes == 2 {
                return address_bytes[0] == pattern_value && address_bytes[1] == pattern_value;
            }
            
            // Slow path for more bytes
            for i in 0..num_bytes {
                if i >= address_bytes.len() || address_bytes[i] != pattern_value {
                    return false;
                }
            }
            return true;
        },
        5 => {
            // Complex pattern with different leading and trailing bytes
            let leading_byte = pattern_value;
            let trailing_byte = (pattern_value as u32 >> 8) as u8;
            
            // Fast path - check first and last byte directly
            if address_bytes[0] != leading_byte || address_bytes[address_bytes.len() - 1] != trailing_byte {
                return false;
            }
            
            // Additional checks for multiple leading bytes if needed
            let num_bytes = pattern_length as usize;
            if num_bytes > 1 {
                for i in 1..num_bytes {
                    if i >= address_bytes.len() || address_bytes[i] != leading_byte {
                        return false;
                    }
                }
            }
            
            return true;
        },
        _ => {
            // For unrecognized pattern flags, default to matching "abcd" prefix and "ef" suffix
            let address_hex = hex::encode(address_bytes);
            return address_hex.starts_with("abcd") && address_hex.ends_with("ef");
        }
    }
}
