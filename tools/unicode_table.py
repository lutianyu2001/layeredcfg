"""Write the core's Unicode table (Rust) from the unicodedata of the Python that runs this: its decimal digits and its
whitespace, so that the core reads numbers and strips text as that Python does (int(), float(), str.strip()).

    python tools/unicode_table.py OUT

The core's build script (crates/layeredcfg/build.rs) runs it with the Python it is given, for a wheel the one the
wheel is for.
"""

import sys
import unicodedata
from pathlib import Path


def ranges(codes: list[int]) -> list[tuple[int, int]]:
    """The codes as runs of consecutive ones, first and last."""
    runs: list[tuple[int, int]] = []
    for code in codes:
        if runs and runs[-1][1] == code - 1:
            runs[-1] = (runs[-1][0], code)
        else:
            runs.append((code, code))
    return runs


def table() -> str:
    zeros, spaces, digits = [], [], 0
    for code in range(0x110000):
        char = chr(code)
        value = unicodedata.decimal(char, -1)
        if value >= 0:
            digits += 1
            if value == 0:
                zeros.append(code)
        if char.isspace():
            spaces.append(code)
    # Unicode encodes decimal digits in runs of ten, zero to nine, which the core's lookup relies on.
    for zero in zeros:
        if [unicodedata.decimal(chr(zero + k), -1) for k in range(10)] != list(range(10)):
            sys.exit(f"U+{zero:04X} does not begin a run of ten digits")
    if digits != 10 * len(zeros):
        sys.exit("a decimal digit outside the runs of ten")
    # Matches of ranges rather than tables searched, so that there is no loop for Kani to unwind.
    space_arms = "\n        | ".join(
        f"0x{first:04X}" if first == last else f"0x{first:04X}..=0x{last:04X}" for first, last in ranges(spaces)
    )
    digit_arms = "\n".join(f"        0x{zero:04X}..=0x{zero + 9:04X} => Some(0x{zero:04X})," for zero in zeros)
    return (
        f"// Written by tools/unicode_table.py from Python's unicodedata (Unicode {unicodedata.unidata_version}); "
        "not to be edited.\n\n"
        "/// Whether `str.isspace()` takes the character for whitespace.\n"
        "pub(crate) fn is_space(code: u32) -> bool {\n"
        f"    matches!(\n        code,\n        {space_arms}\n    )\n}}\n\n"
        "/// The zero of the run of ten decimal digits the character is in, nothing for a character that is no digit.\n"
        "pub(crate) fn digit_zero(code: u32) -> Option<u32> {\n"
        f"    match code {{\n{digit_arms}\n        _ => None,\n    }}\n}}\n"
    )


if __name__ == "__main__":
    out = Path(sys.argv[1])
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(table())
