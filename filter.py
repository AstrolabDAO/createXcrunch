import hashlib
import sys
import re
from web3 import Web3

# first argument is the file to read
filename = sys.argv[1] if len(sys.argv) > 1 else 'output.txt'
# second argument is the regex pattern (optional)
pattern = sys.argv[2] if len(sys.argv) > 2 else None

with open(filename, 'r') as file:
    lines = file.readlines()

# Sort the lines based on the suffix of the address
sorted_lines = sorted(lines, key=lambda line: line.strip().split(' => ')[1][-4:])

# json start
print("{")
for line in sorted_lines:
    parts = line.strip().split(' => ')
    if len(parts) == 2:
        address = Web3.to_checksum_address(parts[1])
        if pattern is None or re.search(pattern, address):
            print(f'\t"{parts[0]}": "{address}",')
    else:
        raise Exception(f'Invalid line: {line} - {parts}')
# json end
print("}")
