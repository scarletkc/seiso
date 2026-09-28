"""Release versions shared by Cargo/npm and Python distribution metadata."""

import re


VERSION = re.compile(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-(alpha|beta|rc)\.(0|[1-9][0-9]*))?")


def parse_version(version):
    match = VERSION.fullmatch(version)
    if not match:
        raise ValueError("Expected MAJOR.MINOR.PATCH, optionally followed by -alpha.N, -beta.N, or -rc.N")
    major, minor, patch, stage, number = match.groups()
    return (int(major), int(minor), int(patch)), stage, int(number) if number is not None else 0


def version_key(version):
    core, stage, number = parse_version(version)
    return (*core, {"alpha": 0, "beta": 1, "rc": 2, None: 3}[stage], number)


def python_version(version):
    core, stage, number = parse_version(version)
    base = ".".join(map(str, core))
    return base + ({"alpha": "a", "beta": "b", "rc": "rc"}[stage] + str(number) if stage else "")
