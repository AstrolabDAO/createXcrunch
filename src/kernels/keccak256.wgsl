const KECCAK_ROUND = 24u;
const KECCAK256_INPUT_BUF_SIZE:u32 = 32; // 32 * 32bit
const KECCAK256_OUTPUT_SIZE:u32 = 8; // 8 * 32bit

// Keccak-256 round constants - merged for better access patterns
const SHA3_IOTA_H = array<u32, 24>(
1, 32898, 32906, 2147516416, 32907, 2147483649, 2147516545, 32777, 138, 136, 2147516425, 2147483658, 2147516555, 139, 32905, 32771, 32770, 128, 32778, 2147483658, 2147516545, 32896, 2147483649, 2147516424
);

const SHA3_IOTA_L = array<u32, 24>(
0, 0, 2147483648, 2147483648, 0, 0, 2147483648, 2147483648, 0, 0, 0, 0, 0, 2147483648, 2147483648, 2147483648, 2147483648, 2147483648, 0, 2147483648, 2147483648, 2147483648, 0, 2147483648
);

// Optimized rotation pattern based on OpenCL implementation
// Combine rotation indices for faster access
const ROTATION_INDICES = array<u32, 48>(
// First 24 are rotation values (from SHA3_ROTL)
1, 3, 6, 10, 15, 21, 28, 36, 45, 55, 2, 14, 27, 41, 56, 8, 25, 43, 62, 18, 39, 61, 20, 44,
// Next 24 are permutation values (from SHA3_PI)
20, 14, 22, 34, 36, 6, 10, 32, 16, 42, 48, 8, 30, 46, 38, 26, 24, 4, 40, 28, 44, 18, 12, 2
);

// Optimized rotate left function for 64-bit values (represented as 2x32)
fn rotl64(x_high: u32, x_low: u32, s: u32) -> vec2<u32> {
    var result: vec2<u32>;
    
    if (s >= 32u) {
        // For shifts >= 32, we swap high and low
        let actual_shift = s - 32u;
        result.x = (x_low << actual_shift) | (x_high >> (32u - actual_shift));
        result.y = (x_high << actual_shift) | (x_low >> (32u - actual_shift));
    } else {
        // For shifts < 32, we keep components in same place
        result.x = (x_high << s) | (x_low >> (32u - s));
        result.y = (x_low << s) | (x_high >> (32u - s));
    }
    
    return result;
}

// Optimized Keccak-f[1600] using 64-bit operations where possible
fn keccak_f1600(state: ptr<function, array<u32, 50>>) {
    var B: array<u32, 10>;
    var t_high: u32;
    var t_low: u32;
    var rotated: vec2<u32>;
    
    // Optimize for GPU by removing nested loops where possible
    for (var round: u32 = 0u; round < KECCAK_ROUND; round = round + 1u) {
        // Theta step
        // Calculate column parity
        for (var x: u32 = 0u; x < 5u; x = x + 1u) {
            B[x*2] = (*state)[x*2] ^ (*state)[(x+5u)*2] ^ (*state)[(x+10u)*2] ^ (*state)[(x+15u)*2] ^ (*state)[(x+20u)*2];
            B[x*2+1] = (*state)[x*2+1] ^ (*state)[(x+5u)*2+1] ^ (*state)[(x+10u)*2+1] ^ (*state)[(x+15u)*2+1] ^ (*state)[(x+20u)*2+1];
        }
        
        // Theta: apply parity to each column
        for (var x: u32 = 0u; x < 5u; x = x + 1u) {
            // Calculate theta effect
            let idx1 = ((x + 1u) % 5u) * 2u;
            let idx2 = ((x + 4u) % 5u) * 2u;
            
            rotated = rotl64(B[idx1], B[idx1+1], 1u);
            t_high = B[idx2] ^ rotated.x;
            t_low = B[idx2+1] ^ rotated.y;
            
            // Apply theta effect to all words in this column
            for (var y: u32 = 0u; y < 5u; y = y + 1u) {
                (*state)[(y*5u + x)*2] ^= t_high;
                (*state)[(y*5u + x)*2+1] ^= t_low;
            }
        }
        
        // Rho and Pi steps combined
        // Store first word for later
        t_high = (*state)[2];
        t_low = (*state)[3];
        
        // Apply Rho and Pi to all words
        for (var t: u32 = 0u; t < 24u; t = t + 1u) {
            let rotation = ROTATION_INDICES[t];
            let permutation = ROTATION_INDICES[t + 24u];
            
            // Rotate current word
            rotated = rotl64(t_high, t_low, rotation);
            
            // Store rotated value temporarily
            let temp_high = rotated.x;
            let temp_low = rotated.y;
            
            // Get next word from permutation
            t_high = (*state)[permutation*2];
            t_low = (*state)[permutation*2+1];
            
            // Update state with rotated word
            (*state)[permutation*2] = temp_high;
            (*state)[permutation*2+1] = temp_low;
        }
        
        // Chi step
        // Process each row
        for (var y: u32 = 0u; y < 5u; y = y + 1u) {
            // Store row in B
            for (var x: u32 = 0u; x < 5u; x = x + 1u) {
                B[x*2] = (*state)[(y*5u + x)*2];
                B[x*2+1] = (*state)[(y*5u + x)*2+1];
            }
            
            // Apply Chi transformation
            for (var x: u32 = 0u; x < 5u; x = x + 1u) {
                let idx1 = ((x + 1u) % 5u) * 2u;
                let idx2 = ((x + 2u) % 5u) * 2u;
                
                (*state)[(y*5u + x)*2] = B[x*2] ^ ((~B[idx1]) & B[idx2]);
                (*state)[(y*5u + x)*2+1] = B[x*2+1] ^ ((~B[idx1+1]) & B[idx2+1]);
            }
        }
        
        // Iota step: XOR round constant
        (*state)[0] ^= SHA3_IOTA_H[round];
        (*state)[1] ^= SHA3_IOTA_L[round];
    }
}

// The rest of the keccak256 implementation using the optimized keccak_f1600
fn keccak256(input: ptr<function, array<u32, KECCAK256_INPUT_BUF_SIZE>>, input_len:u32, output: ptr<function, array<u32, KECCAK256_OUTPUT_SIZE>>) {
    var state: array<u32, 50>; // Initialize to zeros
    
    // Pre-process input data
    // XOR input data into state
    for (var i: u32 = 0u; i < input_len && i < 17u*2u; i = i + 1u) {
        state[i] = (*input)[i];
    }
    
    // Add padding: first byte of padding is 0x01
    let end_idx = input_len; 
    if (end_idx < 17u*2u) {
        state[end_idx] = state[end_idx] ^ 0x01u;
        
        // Add end padding byte (0x80) at end of block
        state[17u*2u-1u] = state[17u*2u-1u] ^ 0x80000000u;
    }
    
    // Apply the permutation function
    keccak_f1600(&state);
    
    // Copy output to result
    for (var i: u32 = 0u; i < KECCAK256_OUTPUT_SIZE; i = i + 1u) {
        (*output)[i] = state[i];
    }
}

@group(0) @binding(0) var<storage, read_write> messages: array<u32>;
@group(0) @binding(1) var<storage, read> num_messages: u32;
@group(0) @binding(2) var<storage, read> message_sizes: array<u32>;
@group(0) @binding(3) var<storage, read_write> hashes: array<u32>;
@group(0) @binding(4) var<storage, read_write> log_buffer: array<u32>;
@group(0) @binding(5) var<storage, read> pattern_config: array<u32, 4>; // [pattern_value, pattern_type, pattern_length, reserved]

// Helper function to extract a byte from the output buffer
fn get_byte_from_hash(output: ptr<function, array<u32, KECCAK256_OUTPUT_SIZE>>, byte_idx: u32) -> u32 {
    let word_idx = byte_idx / 4u;
    let byte_offset = byte_idx % 4u;
    let shift_amount = 24u - (byte_offset * 8u); // Convert byte offset to bit shift
    return ((*output)[word_idx] >> shift_amount) & 0xFFu;
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let index = global_id.x;
    
    // Early exit for threads beyond our work size
    if (index >= num_messages) {
        return;
    }
    
    // Setup input buffer
    var input: array<u32, KECCAK256_INPUT_BUF_SIZE>;
    var output: array<u32, KECCAK256_OUTPUT_SIZE>;
    
    // Get the base nonce from the messages buffer (global nonce)
    let base_nonce_low = messages[0];  // Low 32 bits of nonce
    let base_nonce_high = messages[1]; // High 32 bits of nonce
    
    // Add the thread index to create a unique nonce for each thread
    // This effectively processes nonces from base_nonce to base_nonce + num_messages - 1
    let nonce_low = base_nonce_low + index;
    // Handle carry for overflow using select() instead of if expression
    let nonce_high = base_nonce_high + select(0u, 1u, nonce_low < base_nonce_low);
    
    // Set the input with the combined nonce
    input[0] = nonce_low;  // Low 32 bits of nonce
    input[1] = nonce_high; // High 32 bits of nonce
    
    // Zero out the rest of the input
    for (var i: u32 = 2u; i < KECCAK256_INPUT_BUF_SIZE; i = i + 1u) {
        input[i] = 0u;
    }
    
    // Perform keccak256 hash
    keccak256(&input, 8u, &output); // Using 8 as input_len (32 bytes, 8 u32s)
    
    // Just store salt and hash in the output buffer for CPU validation
    // Save the nonce along with its hash
    let base_index = index * (2 + KECCAK256_OUTPUT_SIZE); // 2 words for nonce, 8 words for hash
    
    // Store nonce
    hashes[base_index] = nonce_low;
    hashes[base_index + 1] = nonce_high;
    
    // Store hash
    for (var i: u32 = 0u; i < KECCAK256_OUTPUT_SIZE; i = i + 1u) {
        hashes[base_index + 2 + i] = output[i];
    }
}
