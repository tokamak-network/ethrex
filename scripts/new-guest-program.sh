#!/usr/bin/env bash
set -euo pipefail

# Guest Program Scaffold Generator
# Usage: ./scripts/new-guest-program.sh <program-name>
# Example: ./scripts/new-guest-program.sh my-program

if [ $# -lt 1 ]; then
    echo "Usage: $0 <program-name>"
    echo "Example: $0 my-custom-program"
    echo ""
    echo "This creates:"
    echo "  - crates/guest-program/src/programs/<name>.rs   (GuestProgram trait impl)"
    echo "  - Registers the program in programs/mod.rs"
    exit 1
fi

PROGRAM_NAME="$1"
# Validate: lowercase, numbers, hyphens only
if ! echo "$PROGRAM_NAME" | grep -qE '^[a-z][a-z0-9-]{1,62}[a-z0-9]$'; then
    echo "Error: program name must be 3-64 chars, lowercase letters, numbers, and hyphens"
    exit 1
fi

# Convert to Rust identifiers
SNAKE_NAME=$(echo "$PROGRAM_NAME" | tr '-' '_')
PASCAL_NAME=$(echo "$PROGRAM_NAME" | sed -E 's/(^|-)([a-z])/\U\2/g')

PROGRAMS_DIR="crates/guest-program/src/programs"
MOD_FILE="$PROGRAMS_DIR/mod.rs"

# Check we're in repo root
if [ ! -f "$MOD_FILE" ]; then
    echo "Error: must be run from the ethrex repository root"
    echo "Expected: $MOD_FILE"
    exit 1
fi

# Check for duplicates
if [ -f "$PROGRAMS_DIR/${SNAKE_NAME}.rs" ]; then
    echo "Error: $PROGRAMS_DIR/${SNAKE_NAME}.rs already exists"
    exit 1
fi

echo "Creating guest program: $PROGRAM_NAME"
echo "  Struct: ${PASCAL_NAME}GuestProgram"
echo "  Module: $SNAKE_NAME"

# Determine next available type ID
# Look for the highest program_type_id in existing programs
NEXT_TYPE_ID=$(grep -h 'fn program_type_id' "$PROGRAMS_DIR"/*.rs 2>/dev/null \
    | grep -oE '[0-9]+' | sort -n | tail -1)
NEXT_TYPE_ID=$((NEXT_TYPE_ID + 1))

echo "  Type ID: $NEXT_TYPE_ID"

# Create the program source file
cat > "$PROGRAMS_DIR/${SNAKE_NAME}.rs" << RUST_EOF
//! Guest Program: $PROGRAM_NAME
//!
//! This is a scaffold for a custom guest program.
//! Implement the \`elf()\` and \`vk_bytes()\` methods once ELF binaries are compiled.

use crate::traits::{GuestProgram, GuestProgramError};

pub struct ${PASCAL_NAME}GuestProgram;

impl GuestProgram for ${PASCAL_NAME}GuestProgram {
    fn program_id(&self) -> &str {
        "$PROGRAM_NAME"
    }

    fn program_type_id(&self) -> u8 {
        $NEXT_TYPE_ID
    }

    fn elf(&self, _backend: &str) -> Option<&[u8]> {
        // TODO: Return compiled ELF bytes for each backend
        // Example:
        //   backends::SP1 => Some(include_bytes!("...")),
        //   backends::RISC0 => Some(include_bytes!("...")),
        None
    }

    fn vk_bytes(&self, _backend: &str) -> Option<Vec<u8>> {
        // TODO: Return verification key bytes for each backend
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn program_id_is_correct() {
        let prog = ${PASCAL_NAME}GuestProgram;
        assert_eq!(prog.program_id(), "$PROGRAM_NAME");
    }

    #[test]
    fn program_type_id_is_correct() {
        let prog = ${PASCAL_NAME}GuestProgram;
        assert_eq!(prog.program_type_id(), $NEXT_TYPE_ID);
    }

    #[test]
    fn unknown_backend_returns_none() {
        let prog = ${PASCAL_NAME}GuestProgram;
        assert!(prog.elf("unknown").is_none());
        assert!(prog.vk_bytes("unknown").is_none());
    }

    #[test]
    fn serialize_input_is_identity() {
        let prog = ${PASCAL_NAME}GuestProgram;
        let data = vec![1, 2, 3];
        assert_eq!(prog.serialize_input(&data).unwrap(), data);
    }
}
RUST_EOF

echo "  Created: $PROGRAMS_DIR/${SNAKE_NAME}.rs"

# Register in mod.rs
# Add module declaration and re-export
# We need to add:
#   mod <snake_name>;
#   pub use <snake_name>::<PascalName>GuestProgram;

# Find the last "mod" line and add after it
if ! grep -q "mod ${SNAKE_NAME};" "$MOD_FILE"; then
    # Add mod declaration after the last existing mod line
    LAST_MOD_LINE=$(grep -n "^mod " "$MOD_FILE" | tail -1 | cut -d: -f1)
    sed -i '' "${LAST_MOD_LINE}a\\
mod ${SNAKE_NAME};" "$MOD_FILE"
    echo "  Added: mod ${SNAKE_NAME}; to $MOD_FILE"
fi

if ! grep -q "pub use ${SNAKE_NAME}::${PASCAL_NAME}GuestProgram;" "$MOD_FILE"; then
    # Add pub use after the last existing pub use line
    LAST_USE_LINE=$(grep -n "^pub use " "$MOD_FILE" | tail -1 | cut -d: -f1)
    sed -i '' "${LAST_USE_LINE}a\\
pub use ${SNAKE_NAME}::${PASCAL_NAME}GuestProgram;" "$MOD_FILE"
    echo "  Added: pub use ${SNAKE_NAME}::${PASCAL_NAME}GuestProgram; to $MOD_FILE"
fi

echo ""
echo "Done! Next steps:"
echo "  1. Implement elf() and vk_bytes() in $PROGRAMS_DIR/${SNAKE_NAME}.rs"
echo "  2. Register in prover: crates/l2/prover/src/prover.rs (create_default_registry)"
echo "  3. Build: GUEST_PROGRAMS=evm-l2,$PROGRAM_NAME make l2-sp1"
echo "  4. Run tests: cargo test -p ethrex-guest-program"
