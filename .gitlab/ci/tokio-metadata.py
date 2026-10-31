"""Validate Cargo's standalone Tokio graph and preserve configured compiler flags."""

import json
import os
from pathlib import Path
import shlex
import sys
import tomllib


def main():
    mode, source, target = sys.argv[1:4]
    if mode in ("flags", "preserve-flags"):
        if "CARGO_ENCODED_RUSTFLAGS" in os.environ:
            flags = os.environ["CARGO_ENCODED_RUSTFLAGS"].split("\x1f")
        elif "RUSTFLAGS" in os.environ:
            flags = shlex.split(os.environ["RUSTFLAGS"])
        else:
            config = tomllib.loads(Path(source).read_text())
            target_env = f"CARGO_TARGET_{target.upper().replace('-', '_')}_RUSTFLAGS"
            flags = os.environ.get(target_env)
            if flags is None:
                flags = config.get("target", {}).get(target, {}).get("rustflags")
            if flags is None:
                flags = config.get("build", {}).get("rustflags", [])
            if isinstance(flags, str):
                flags = shlex.split(flags)
        flags = [flag for flag in flags if flag]
        if mode == "flags":
            flags.extend(["--cfg", "tokio_unstable"])
        sys.stdout.write("\x1f".join(flags))
    elif mode == "graph":
        metadata = json.loads(Path(source).read_text())
        manifest = Path(sys.argv[4]).resolve()
        packages = [package for package in metadata["packages"] if package["name"] == "tokio"]
        if len(packages) != 1:
            raise ValueError("standalone graph must contain exactly one Tokio identity")
        package = packages[0]
        if Path(package["manifest_path"]).resolve() != manifest or package["source"] is not None:
            raise ValueError("Tokio must resolve to the exact vendored manifest")
        if metadata["resolve"]["root"] != package["id"]:
            raise ValueError("standalone graph root must be vendored Tokio")
        node = next(node for node in metadata["resolve"]["nodes"] if node["id"] == package["id"])
        print(f"target={target}")
        print(f"tokio_id={package['id']}")
        print(f"manifest={manifest}")
        print(f"features={','.join(sorted(node['features']))}")
        print("tokio_identities=1")
    else:
        raise ValueError("expected flags, preserve-flags or graph mode")


if __name__ == "__main__":
    main()
