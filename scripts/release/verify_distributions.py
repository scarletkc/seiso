"""Check distribution metadata and declared license files before uploading."""

from email.parser import BytesParser
from pathlib import Path, PurePosixPath
import sys
import tarfile
import tomllib
import zipfile

from .versions import python_version


def verify(directory: Path, root: Path) -> None:
    version = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))["package"]["version"]
    expected_license = (root / "LICENSE").read_text(encoding="utf-8").strip()
    distributions = sorted(directory.glob("*.whl")) + sorted(directory.glob("*.tar.gz"))
    if not distributions:
        raise ValueError(f"No distributions found in {directory}")
    for distribution in distributions:
        if distribution.suffix == ".whl":
            with zipfile.ZipFile(distribution) as archive:
                names = archive.namelist()
                metadata_paths = [name for name in names if name.endswith(".dist-info/METADATA")]
                if len(metadata_paths) != 1:
                    raise ValueError(f"{distribution.name}: expected one METADATA file")
                metadata_path = metadata_paths[0]
                metadata = BytesParser().parsebytes(archive.read(metadata_path))
                prefix = str(PurePosixPath(metadata_path).parent / "licenses")
                validate(metadata, prefix, archive.read, version, expected_license)
        else:
            with tarfile.open(distribution) as archive:
                metadata_paths = [name for name in archive.getnames() if len(PurePosixPath(name).parts) == 2 and name.endswith("/PKG-INFO")]
                if len(metadata_paths) != 1:
                    raise ValueError(f"{distribution.name}: expected one top-level PKG-INFO file")
                metadata_path = metadata_paths[0]
                def read(name):
                    member = archive.extractfile(name)
                    if member is None:
                        raise ValueError(f"{distribution.name}: {name} is not a regular file")
                    return member.read()
                metadata = BytesParser().parsebytes(read(metadata_path))
                validate(metadata, str(PurePosixPath(metadata_path).parent), read, version, expected_license)
        print(f"Verified {distribution.name}")


def validate(metadata, prefix, read, version, expected_license):
    if metadata["Name"] != "seiso" or metadata["Version"] != python_version(version):
        raise ValueError("Distribution name/version differs from the Cargo package")
    licenses = metadata.get_all("License-File", [])
    if not licenses:
        raise ValueError("Distribution has no declared license file")
    for license_file in licenses:
        contents = read(f"{prefix}/{license_file}").decode("utf-8").replace("\r\n", "\n").replace("\r", "\n").strip()
        if contents != expected_license:
            raise ValueError(f"{license_file} differs from the repository license")


if __name__ == "__main__":
    verify(Path(sys.argv[1]), Path(__file__).resolve().parents[2])
