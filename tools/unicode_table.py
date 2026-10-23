"""Write the core's Unicode table from the unicodedata of the Python that runs this: its decimal digits and its
whitespace, so that the core reads numbers and strips text as that Python does (int(), float(), str.strip()).

    python tools/unicode_table.py OUT

The build runs it with the Python it finds, for a wheel the one the wheel is for.
"""

import sys
import unicodedata
from pathlib import Path


def rows(codes: list[int]) -> str:
    return "\n".join(
        "    " + ", ".join(f"0x{code:04X}" for code in codes[at : at + 8]) + "," for at in range(0, len(codes), 8)
    )


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
    return (
        f"// Written by tools/unicode_table.py from Python's unicodedata (Unicode {unicodedata.unidata_version}); "
        "not to be edited.\n\n"
        "// The zero of each run of ten decimal digits, ascending.\n"
        f"constexpr char32_t digit_zeros[] = {{\n{rows(zeros)}\n}};\n\n"
        "// The characters str.isspace() takes for whitespace, ascending.\n"
        f"constexpr char32_t spaces[] = {{\n{rows(spaces)}\n}};\n"
    )


if __name__ == "__main__":
    out = Path(sys.argv[1])
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(table())
