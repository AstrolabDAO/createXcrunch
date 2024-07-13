#!/bin/bash
DEPLOYER=${DEPLOYER:-0xbEbA0C3A4296DF22ba02D0e825BC7e8e9f5b16B0} # default to deployer 1
./target/release/createxcrunch create3 --caller $DEPLOYER --matching 0000XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXaaaX
