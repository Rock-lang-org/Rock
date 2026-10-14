#!/usr/bin/env python3
"""
Script to mechanically split the resolve/mod.rs file into smaller temporary files.
Each temp file will be ~500 lines and split at logical boundaries (function/struct boundaries).

Usage:
    python3 split_resolve.py

This will create:
    lib/src/resolve/temp_XX_name.rs files
"""

import re
import os
from pathlib import Path

# Configuration
SOURCE_FILE = Path(__file__).parent.parent / "lib/src/resolve/mod.rs"
OUTPUT_DIR = Path(__file__).parent.parent / "lib/src/resolve"
MAX_LINES_PER_FILE = 500

# Section definitions: (start_line, end_line, name, description)
# These are approximate boundaries based on logical groupings
SECTIONS = [
    # Header and imports (lines 1-21)
    (1, 21, "header", "Module header and imports"),

    # ResolveError struct and impls (lines 23-59)
    (23, 60, "resolve_error", "ResolveError struct and impls"),

    # InferenceEngine (lines 61-261)
    (61, 261, "inference_engine", "InferenceEngine struct and impls"),

    # Scope (lines 262-297)
    (262, 297, "scope", "Scope struct and impls"),

    # Helper functions and Lowerer struct (lines 298-367)
    (298, 367, "lowerer_struct", "Lowerer struct and helper functions"),

    # Lowerer impl: Type helpers (lines 368-424)
    (368, 424, "lowerer_type_helpers", "Lowerer type helper methods"),

    # Lowerer impl: Constructors (lines 425-490)
    (425, 490, "lowerer_constructors", "Lowerer constructors and error methods"),

    # Lowerer impl: Main entry points (lines 491-590)
    (491, 590, "lowerer_entry_points", "Lowerer main entry points"),

    # Lowerer impl: Std/crate registration (lines 591-822)
    (591, 822, "lowerer_registration", "Std and crate registration"),

    # Lowerer impl: Crate body lowering (lines 823-1102)
    (823, 1102, "lowerer_crate_bodies", "Crate body lowering"),

    # Lowerer impl: Declaration collection (lines 1103-1466)
    (1103, 1466, "lowerer_collect_decls", "Declaration collection"),

    # Lowerer impl: Import/module handling (lines 1467-1685)
    (1467, 1685, "lowerer_imports", "Import and module handling"),

    # Lowerer impl: Type collection (lines 1686-1867)
    (1686, 1867, "lowerer_collect_types", "Struct and enum collection"),

    # Lowerer impl: Trait/function collection (lines 1868-2008)
    (1868, 2008, "lowerer_collect_sigs", "Trait and function signature collection"),

    # Lowerer impl: Impl collection (lines 2009-2257)
    (2009, 2257, "lowerer_collect_impls", "Impl collection and trait conformance"),

    # Lowerer impl: Function header lowering (lines 2258-2338)
    (2258, 2338, "lowerer_func_headers", "Function header lowering"),

    # Lowerer impl: Module/function body lowering (lines 2339-2499)
    (2339, 2499, "lowerer_bodies", "Module and function body lowering"),

    # Lowerer impl: Trait default bodies (lines 2500-2733)
    (2500, 2733, "lowerer_trait_bodies", "Trait default and impl bodies"),

    # Lowerer impl: Statement lowering (lines 2734-2933)
    (2734, 2933, "lowerer_statements", "Block and statement lowering"),

    # Lowerer impl: Expression lowering part 1 (lines 2934-3320)
    (2934, 3320, "lowerer_expr_1", "Expression lowering (precedence, literals)"),

    # Lowerer impl: Expression lowering part 2 (lines 3321-3867)
    (3321, 3867, "lowerer_expr_2", "Expression lowering (paths, instances, lambdas, control flow)"),

    # Lowerer impl: Method call handling (lines 3868-4168)
    (3868, 4168, "lowerer_method_calls", "Method call and secondary expr handling"),

    # Lowerer impl: Type lowering (lines 4169-4242)
    (4169, 4242, "lowerer_types", "ParseType lowering"),

    # Lowerer impl: Finalization (lines 4243-4453)
    (4243, 4453, "lowerer_finalize", "Type finalization"),

    # Lowerer impl: Type variable collection (lines 4454-4574)
    (4454, 4574, "lowerer_type_vars", "Type variable collection"),

    # Lowerer impl: Generalization part 1 (lines 4575-4892)
    (4575, 4892, "lowerer_generalize_1", "Type generalization"),

    # Lowerer impl: Generalization part 2 (lines 4893-5113)
    (4893, 5113, "lowerer_generalize_2", "Type replacement with generics"),

    # Lowerer impl: Generalization part 3 (lines 5114-5297)
    (5114, 5297, "lowerer_generalize_3", "Composite type replacement"),

    # Public functions (lines 5298-5334)
    (5298, 5334, "public_api", "Public API functions"),

    # Intrinsic helpers (lines 5335-end)
    (5335, 6000, "intrinsics", "Intrinsic function helpers"),
]


def find_function_boundary(lines, start_line, max_line):
    """Find a good boundary point near max_line (at a function/struct boundary)."""
    if start_line >= len(lines):
        return len(lines)

    target = min(start_line + max_line, len(lines))

    # Look backwards from target for a boundary
    for i in range(target, start_line, -1):
        if i >= len(lines):
            continue
        line = lines[i]
        # Check for function/struct/impl boundaries
        if re.match(r'^(pub )?(struct|impl|fn |pub fn |trait |enum )', line.strip()):
            return i

    # If no boundary found, just use max_line
    return target


def extract_section(lines, start, end):
    """Extract lines from start to end (0-indexed, end exclusive)."""
    return lines[start-1:end]


def create_temp_file(output_dir, index, name, content, description):
    """Create a temporary file with the given content."""
    filename = f"temp_{index:02d}_{name}.rs"
    filepath = output_dir / filename

    # Add header comment
    header = f"// === TEMPORARY FILE: {name} ===\n"
    header += f"// {description}\n"
    header += f"// Lines from original: will be determined during extraction\n"
    header += f"// This file is for analysis purposes during refactoring.\n\n"

    with open(filepath, 'w') as f:
        f.write(header + ''.join(content))

    return filename


def main():
    print(f"Reading {SOURCE_FILE}...")
    with open(SOURCE_FILE, 'r') as f:
        lines = f.readlines()

    total_lines = len(lines)
    print(f"Total lines: {total_lines}")

    # Create temp directory if needed
    temp_dir = OUTPUT_DIR / "temp"
    temp_dir.mkdir(exist_ok=True)

    created_files = []

    for idx, (start, end, name, description) in enumerate(SECTIONS):
        # Adjust end to not exceed file length
        actual_end = min(end, total_lines)
        if start > total_lines:
            continue

        content = extract_section(lines, start, actual_end)
        filename = create_temp_file(temp_dir, idx, name, content, description)
        created_files.append((filename, start, actual_end, len(content)))
        print(f"Created {filename}: lines {start}-{actual_end} ({len(content)} lines)")

    # Create an index file
    index_path = temp_dir / "INDEX.md"
    with open(index_path, 'w') as f:
        f.write("# Resolve Module Split Index\n\n")
        f.write("This directory contains temporary split files for the resolve module.\n\n")
        f.write("## Files\n\n")
        f.write("| File | Original Lines | Lines | Description |\n")
        f.write("|------|----------------|-------|-------------|\n")
        for filename, start, end, line_count in created_files:
            f.write(f"| {filename} | {start}-{end} | {line_count} | - |\n")

        f.write("\n## Usage\n\n")
        f.write("1. Each file contains a logically grouped section of the original module\n")
        f.write("2. Use these files to understand the structure before creating final modules\n")
        f.write("3. Delete this temp directory after refactoring is complete\n")

    print(f"\nCreated {len(created_files)} temporary files in {temp_dir}")
    print(f"See {index_path} for index")


if __name__ == "__main__":
    main()
