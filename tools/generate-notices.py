#!/usr/bin/env python3
"""Generate offline-reproducible notices for the Windows Cargo dependency graph.

Python 3.11+ standard library only. Initial upstream license retrieval is opt-in.
Normal builds do not run this tool or need access to the upstream repositories.
"""

from __future__ import annotations

import argparse
import hashlib
import itertools
import json
import pathlib
import re
import subprocess
import sys
import tomllib
import urllib.parse
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parents[1]
TARGET = "x86_64-pc-windows-msvc"
LICENSES = ROOT / "licenses"
MANIFEST = LICENSES / "manifest.json"
NOTICES = ROOT / "THIRD_PARTY_NOTICES.md"
MAX_LICENSE_BYTES = 300_000
FONT_NOTICES = (
    "fonts/Hack-Regular.txt",
    "fonts/OFL.txt",
    "fonts/UFL.txt",
    "fonts/emoji-icon-font-mit-license.txt",
)


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def json_bytes(value: object) -> bytes:
    return (json.dumps(value, ensure_ascii=False, indent=2) + "\n").encode("utf-8")


def within(root: pathlib.Path, relative: str) -> pathlib.Path:
    path = root / relative
    if path.is_symlink() or not path.resolve().is_relative_to(root.resolve()):
        raise ValueError(f"Unsafe notice path: {relative}")
    return path


def checked_text(data: bytes, source: str) -> bytes:
    if not data or len(data) > MAX_LICENSE_BYTES or b"\0" in data:
        raise ValueError(f"Invalid license text size/content: {source}")
    data.decode("utf-8-sig")
    return data


def notice_filename(relative: str) -> str:
    path = pathlib.PurePosixPath(relative)
    if path.suffix.lower() not in (".md", ".txt"):
        path = path.with_name(path.name + ".txt")
    return str(path)


def license_options(expression: str) -> list[set[str]]:
    """Evaluate SPDX AND/OR groupings without silently accepting unknown syntax."""
    expression = expression.replace("/", " OR ")
    tokens = re.findall(r"\(|\)|[A-Za-z0-9.+-]+", expression)
    if "".join(tokens) != re.sub(r"\s", "", expression):
        raise ValueError(f"Unsupported license expression: {expression}")
    index = 0

    def atom() -> list[set[str]]:
        nonlocal index
        if index >= len(tokens):
            raise ValueError("Incomplete license expression")
        token = tokens[index]
        index += 1
        if token == "(":
            options = disjunction()
            if index >= len(tokens) or tokens[index] != ")":
                raise ValueError("Unclosed license expression")
            index += 1
            return options
        if token in ("AND", "OR", "WITH", ")"):
            raise ValueError(f"Unsupported license token: {token}")
        return [{token}]

    def conjunction() -> list[set[str]]:
        nonlocal index
        options = atom()
        while index < len(tokens) and tokens[index] == "AND":
            index += 1
            options = [a | b for a, b in itertools.product(options, atom())]
        return options

    def disjunction() -> list[set[str]]:
        nonlocal index
        options = conjunction()
        while index < len(tokens) and tokens[index] == "OR":
            index += 1
            options += conjunction()
        return options

    result = disjunction()
    if index != len(tokens):
        raise ValueError(f"Unsupported license expression: {expression}")
    return result


def recognized_licenses(data: bytes) -> set[str]:
    """Recognize complete notice families used in this reviewed dependency set."""
    text = re.sub(r"\s+", " ", data.decode("utf-8-sig")).lower()
    found = set()
    if "permission is hereby granted" in text and "the software is provided" in text:
        found.add("MIT")
    if "apache license" in text and "version 2.0" in text and "limitation of liability" in text:
        found.add("Apache-2.0")
    if "boost software license" in text and "version 1.0" in text:
        found.add("BSL-1.0")
    if "permission to use, copy, modify, and/or distribute" in text and "the author disclaims" in text:
        found.add("ISC")
    if "this software is provided 'as-is'" in text and "the origin of this software" in text:
        found.add("Zlib")
    if "sil open font license" in text and "version 1.1" in text:
        found.add("OFL-1.1")
    if "ubuntu font licence" in text and "version 1.0" in text:
        found.add("Ubuntu-font-1.0")
    if "unicode license v3" in text:
        found.add("Unicode-3.0")
    return found


def github_upstream(package: dict, package_dir: pathlib.Path, supplemental: str | None = None) -> str | None:
    repository = urllib.parse.urlparse(package.get("repository") or "")
    if repository.netloc.lower() != "github.com":
        return None
    parts = repository.path.strip("/").split("/")
    if len(parts) < 2:
        return None
    project = "/".join(parts[:2]).removesuffix(".git")
    if project.lower() == "emilk/egui":
        filename = "LICENSE-MIT"
    elif project.lower() == "accesskit/accesskit":
        filename = supplemental or (
            "LICENSE-APACHE" if package["name"] == "accesskit_winit" else "LICENSE-MIT"
        )
    elif project.lower() in ("doumanash/clipboard-win", "brendanzab/gl-rs"):
        filename = "LICENSE"
    elif project.lower() == "aclysma/profiling":
        filename = "LICENSE-MIT"
    else:
        return None
    vcs = json.loads((package_dir / ".cargo_vcs_info.json").read_text(encoding="utf-8"))
    commit = vcs["git"]["sha1"]
    if not re.fullmatch(r"[0-9a-f]{40}", commit):
        raise ValueError(f"Invalid source commit for {package['name']}")
    return f"https://raw.githubusercontent.com/{project}/{commit}/{filename}"


def cached_upstream(url: str, old: dict, fetch: bool) -> bytes:
    for package in old.get("packages", []):
        for record in package["files"]:
            if record.get("source_url") == url and record["origin"] == "upstream-commit":
                path = within(ROOT, record["path"])
                if path.exists():
                    data = checked_text(path.read_bytes(), str(path))
                    if digest(data) != record["sha256"]:
                        raise ValueError(f"Cached license hash mismatch: {path}")
                    return data
    if not fetch:
        raise ValueError(f"Missing upstream license cache. Run with --fetch-missing: {url}")
    # Only explicit, commit-pinned upstream license URLs are fetched.
    if not re.fullmatch(
        r"https://raw\.githubusercontent\.com/[^/]+/[^/]+/[0-9a-f]{40}/LICENSE(?:-MIT|-APACHE|\.chromium)?",
        url,
    ):
        raise ValueError(f"Refusing unpinned or unexpected license URL: {url}")
    request = urllib.request.Request(url, headers={"User-Agent": "AulaStudio-license-notices/1"})
    with urllib.request.urlopen(request, timeout=20) as response:
        if response.geturl() != url:
            raise ValueError(f"Unexpected license redirect: {url}")
        return checked_text(response.read(MAX_LICENSE_BYTES + 1), url)


def selected_packages(metadata: dict) -> list[dict]:
    packages = {package["id"]: package for package in metadata["packages"]}
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    root = metadata["resolve"]["root"]
    if packages[root]["name"] != "aula-studio":
        raise ValueError("Metadata must describe the Aula Studio workspace")
    if set(nodes[root]["features"]) - {"default"}:
        raise ValueError("Metadata must use default features, without capture or wireless diagnostics")
    seen = set()
    pending = [root]
    while pending:
        item = pending.pop()
        if item in seen:
            continue
        seen.add(item)
        for dependency in nodes[item]["deps"]:
            if any(kind["kind"] in (None, "build") for kind in dependency["dep_kinds"]):
                pending.append(dependency["pkg"])
    result = [packages[item] for item in seen if item != root]
    for package in result:
        if not (package.get("source") or "").startswith("registry+"):
            raise ValueError(f"Unreviewed non-registry dependency: {package['name']}")
    return sorted(result, key=lambda package: (package["name"], package["version"]))


def registry_texts(package: dict, directory: pathlib.Path) -> dict[str, tuple[bytes, str]]:
    texts = {}
    for path in sorted(directory.rglob("*")):
        name = path.name.lower()
        if path.is_file() and name.startswith(("license", "licence", "copying", "notice", "copyright")):
            if path.suffix.lower() in (".rs", ".c", ".h", ".toml"):
                raise ValueError(f"Review this ambiguous license filename before including it: {path}")
            relative = path.relative_to(directory).as_posix()
            texts[relative] = (checked_text(path.read_bytes(), str(path)), "registry-file")
    if package["name"] == "epaint_default_fonts":
        for relative in FONT_NOTICES:
            path = directory / relative
            texts[relative] = (checked_text(path.read_bytes(), str(path)), "registry-file")
    if package["name"].startswith("accesskit") or package["name"] in ("gl_generator", "khronos_api"):
        # These packages omit their root license files and retain copyright
        # attribution in source headers. Extract only leading legal comments.
        headers: dict[str, list[str]] = {}
        for path in sorted(directory.rglob("*.rs")):
            with path.open(encoding="utf-8") as source:
                prefix = source.read(16_384)
            lines = []
            for line in prefix.splitlines():
                if line.startswith("//") and not line.startswith(("///", "//!")):
                    lines.append(line[2:].removeprefix(" "))
                elif not line.strip():
                    lines.append("")
                else:
                    break
            header = "\n".join(lines).strip()
            if "copyright" in header.lower():
                headers.setdefault(header, []).append(path.relative_to(directory).as_posix())
        if headers:
            content = "\n\n".join(
                "Source files: " + ", ".join(paths) + "\n\n" + header
                for header, paths in sorted(headers.items())
            ) + "\n"
            texts["SOURCE-COPYRIGHT-NOTICES.txt"] = (content.encode("utf-8"), "registry-license-extract")
    # These XML/IDL registry headers contain third-party copyright/permission
    # notices in addition to the Rust crate's declared Apache license.
    if package["name"] == "khronos_api":
        for path in sorted(directory.rglob("*")):
            if not path.is_file() or path.suffix not in (".xml", ".idl"):
                continue
            header = path.read_text(encoding="utf-8")[:16_384]
            blocks = re.findall(r"<!--(.*?)-->|<comment>(.*?)</comment>|/\*(.*?)\*/", header, re.S)
            notices = []
            for captures in blocks:
                block = next((value for value in captures if value), "")
                if "copyright" in block.lower() and (
                    "license" in block.lower() or "permission" in block.lower()
                ):
                    notices.append(block.strip())
            if notices:
                relative = path.relative_to(directory).as_posix()
                texts[relative + ".license.txt"] = (
                    ("\n\n".join(notices) + "\n").encode("utf-8"),
                    "registry-license-extract",
                )
    return texts


def rust_notices(toolchain: pathlib.Path) -> tuple[dict, dict[str, bytes]]:
    compiler = toolchain / "bin" / ("rustc.exe" if sys.platform == "win32" else "rustc")
    version_output = subprocess.check_output(
        [str(compiler), "--version", "--verbose"], text=True, encoding="utf-8"
    )
    values = dict(
        line.split(": ", 1) for line in version_output.splitlines() if ": " in line
    )
    version, commit, host = values["release"], values["commit-hash"], values["host"]
    if not re.fullmatch(r"\d+\.\d+\.\d+", version) or not re.fullmatch(r"[0-9a-f]{40}", commit):
        raise ValueError("Review prerelease or unidentified Rust toolchain licenses explicitly")
    document_dir = toolchain / "share" / "doc" / "rust"
    copyright_file = document_dir / "COPYRIGHT-library.html"
    if not copyright_file.is_file():
        raise ValueError("Rust standard-library copyright notice is missing; install rust-docs")
    paths = [copyright_file] + sorted((document_dir / "licenses").glob("*.txt"))
    if len(paths) < 2:
        raise ValueError("Rust standard-library license text directory is missing")
    record = {
        "name": "Rust Standard Library",
        "version": version,
        "commit": commit,
        "host_toolchain": host,
        "scope": "Official complete standard-library notice; includes other platforms and build dependencies.",
        "files": [],
    }
    output = {}
    for path in paths:
        data = path.read_bytes()
        if not data or len(data) > 2_000_000 or b"\0" in data:
            raise ValueError(f"Invalid Rust legal notice: {path.name}")
        data.decode("utf-8")
        relative = path.relative_to(document_dir).as_posix()
        destination = f"licenses/rust-{version}/{relative}"
        output[destination] = data
        record["files"].append({
            "path": destination,
            "sha256": digest(data),
            "origin": "installed-rust-distribution",
            "source_path": "share/doc/rust/" + relative,
            "source_url": f"https://static.rust-lang.org/dist/rust-docs-{version}-{host}.tar.xz",
        })
    return record, output


def render_notices(manifest: dict) -> bytes:
    lines = [
        "# Third-party notices",
        "",
        f"Generated for Aula Studio's **{TARGET}** default application build.",
        f"This inventory covers **{len(manifest['packages'])} Cargo registry packages**, including",
        "normal dependencies, procedural macros and build dependencies. Optional capture and",
        "wireless diagnostic features are not selected. Other operating systems and the full",
        "cross-platform lockfile are outside this inventory.",
        "",
        "The application embeds egui's default fonts. Their original font notices, including",
        "Hack's Bitstream Vera attribution, are included with `epaint_default_fonts`.",
        "Khronos registry copyright headers are retained as supplemental license extracts.",
        "AccessKit's Chromium-derived definitions retain the supplemental Chromium BSD notice.",
        "The exact installed Rust standard-library notice is included separately:",
        f"[Rust {manifest['rust_standard_library']['version']} standard-library copyrights]"
        f"(licenses/rust-{manifest['rust_standard_library']['version']}/COPYRIGHT-library.html).",
        "Its full upstream inventory also includes other platforms and build dependencies.",
        "Microsoft toolchain/runtime terms are separate from this Cargo and Rust inventory;",
        "this file does not relicense those components.",
        "",
        "Each package's declared license expression is recorded below. The supported license",
        "choice column identifies a complete permitted branch for which the required texts",
        "are included. Other included alternative license texts retain their original wording.",
        "",
        "Full text files and their SHA-256 digests and provenance are recorded in",
        "[licenses/manifest.json](licenses/manifest.json). Registry files come from the exact",
        "versioned crate archive; missing texts come from the package's recorded upstream",
        "Git commit. No dependency source code is copied into this notice bundle.",
        "",
        "Regenerate after a Windows default build with Python 3.11+:",
        "",
        "```powershell",
        "python tools/generate-notices.py --check",
        "```",
        "",
        "The checked-in upstream texts are reused offline. Maintainers explicitly pass",
        "`--fetch-missing` only when a dependency update requires a new upstream license.",
        "`--metadata <file>` accepts Cargo metadata produced with `--locked --offline",
        f"--format-version 1 --filter-platform {TARGET}` and default features.",
        "`--rust-toolchain <directory>` selects the toolchain used for the release; otherwise",
        "`rustc --print sysroot` locates it. The rust-docs component supplies its legal notices.",
        "`--verify-bundle` checks the published notice files without Cargo or its registry.",
        "",
        f"Cargo.lock SHA-256: `{manifest['cargo_lock_sha256']}`",
        "",
        "| Package | Declared license | Supported license choice | Texts |",
        "| --- | --- | --- | --- |",
    ]
    for package in manifest["packages"]:
        texts = ", ".join(
            f"[{pathlib.PurePosixPath(item['path']).name}]({item['path']})"
            for item in package["files"]
        )
        lines.append(
            f"| {package['name']} {package['version']} | {package['license']} | "
            f"{' AND '.join(package['license_choice'])} | {texts} |"
        )
    return ("\n".join(lines) + "\n").encode("utf-8")


def generate(metadata: dict, fetch: bool, toolchain: pathlib.Path) -> tuple[dict, dict[str, bytes]]:
    old = json.loads(MANIFEST.read_text(encoding="utf-8")) if MANIFEST.exists() else {}
    lock_bytes = (ROOT / "Cargo.lock").read_bytes()
    checksums = {
        (item["name"], item["version"]): item["checksum"]
        for item in tomllib.loads(lock_bytes.decode("utf-8"))["package"]
        if "checksum" in item
    }
    manifest = {
        "version": 1,
        "target": TARGET,
        "features": "default",
        "dependency_kinds": ["normal", "build"],
        "cargo_lock_sha256": digest(lock_bytes),
        "packages": [],
    }
    output = {}
    priorities = {"MIT": 0, "Apache-2.0": 1}
    for package in selected_packages(metadata):
        name, version = package["name"], package["version"]
        key = f"{name}-{version}"
        directory = pathlib.Path(package["manifest_path"]).parent
        texts = registry_texts(package, directory)
        available = set().union(*(recognized_licenses(data) for data, _ in texts.values()))
        options = license_options(package["license"])
        upstream = {}
        if name == "accesskit":
            url = github_upstream(package, directory, "LICENSE.chromium")
            if not url:
                raise ValueError("Cannot locate AccessKit's required Chromium copyright notice")
            data = cached_upstream(url, old, fetch)
            if b"Chromium" not in data or b"Redistribution" not in data:
                raise ValueError("Unexpected AccessKit Chromium license text")
            relative = "upstream/LICENSE.chromium"
            texts[relative] = (data, "upstream-commit")
            upstream[relative] = url
        # Font-specific MIT text does not replace the egui crate's own notice.
        if name == "epaint_default_fonts" or not any(option <= available for option in options):
            url = github_upstream(package, directory)
            if not url:
                raise ValueError(f"No complete license texts for {key}: {package['license']}")
            data = cached_upstream(url, old, fetch)
            relative = "upstream/" + url.rsplit("/", 1)[-1]
            texts[relative] = (data, "upstream-commit")
            upstream[relative] = url
            available |= recognized_licenses(data)
        fulfilled = [option for option in options if option <= available]
        if not fulfilled:
            raise ValueError(f"Unfulfilled license for {key}: {package['license']}; found {available}")
        choice = min(fulfilled, key=lambda option: (sum(priorities.get(x, 10) for x in option), sorted(option)))
        record = {
            "name": name,
            "version": version,
            "license": package["license"],
            "license_choice": sorted(choice),
            "repository": package.get("repository"),
            "crate_sha256": checksums[(name, version)],
            "files": [],
        }
        archive_url = f"https://crates.io/api/v1/crates/{name}/{version}/download"
        for relative, (data, origin) in sorted(texts.items()):
            destination = f"licenses/{key}/{notice_filename(relative)}"
            if destination in output and output[destination] != data:
                raise ValueError(f"License filename collision: {destination}")
            output[destination] = data
            record["files"].append({
                "path": destination,
                "sha256": digest(data),
                "origin": origin,
                "source_path": relative,
                "source_url": upstream.get(relative, archive_url),
            })
        manifest["packages"].append(record)
    manifest["rust_standard_library"], runtime_files = rust_notices(toolchain)
    output.update(runtime_files)
    output["THIRD_PARTY_NOTICES.md"] = render_notices(manifest)
    manifest["notices_sha256"] = digest(output["THIRD_PARTY_NOTICES.md"])
    output["licenses/manifest.json"] = json_bytes(manifest)
    return manifest, output


def verify_bundle() -> None:
    manifest = json.loads(MANIFEST.read_text(encoding="utf-8"))
    if manifest["version"] != 1 or manifest["target"] != TARGET:
        raise ValueError("Unsupported notice manifest")
    if digest((ROOT / "Cargo.lock").read_bytes()) != manifest["cargo_lock_sha256"]:
        raise ValueError("Cargo.lock changed; regenerate dependency notices")
    if digest(NOTICES.read_bytes()) != manifest["notices_sha256"]:
        raise ValueError("THIRD_PARTY_NOTICES.md hash mismatch")
    expected = {"licenses/manifest.json"}
    for package in manifest["packages"]:
        if not package["files"]:
            raise ValueError(f"No license texts for {package['name']}")
        for record in package["files"]:
            expected.add(record["path"])
            data = within(ROOT, record["path"]).read_bytes()
            if digest(data) != record["sha256"]:
                raise ValueError(f"License hash mismatch: {record['path']}")
    for record in manifest["rust_standard_library"]["files"]:
        expected.add(record["path"])
        if digest(within(ROOT, record["path"]).read_bytes()) != record["sha256"]:
            raise ValueError(f"Rust license hash mismatch: {record['path']}")
    actual = {path.relative_to(ROOT).as_posix() for path in LICENSES.rglob("*") if path.is_file()}
    if actual != expected:
        raise ValueError("Unexpected or missing license files: " + ", ".join(sorted(actual ^ expected)))
    print(f"Verified notice bundle: {len(manifest['packages'])} Windows Cargo packages.")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--metadata", type=pathlib.Path, help="Windows-filtered Cargo metadata JSON")
    parser.add_argument("--cargo", default="cargo", help="Cargo executable when metadata is not supplied")
    parser.add_argument("--rust-toolchain", type=pathlib.Path, help="Release toolchain with rust-docs installed")
    parser.add_argument("--rustc", default="rustc", help="Rust compiler used to locate the release toolchain")
    parser.add_argument("--fetch-missing", action="store_true", help="Allow pinned upstream license retrieval")
    parser.add_argument("--check", action="store_true", help="Compare regenerated output without changing files")
    parser.add_argument("--verify-bundle", action="store_true", help="Check bundled hashes without Cargo")
    args = parser.parse_args()
    if args.verify_bundle:
        verify_bundle()
        return
    if args.metadata:
        metadata = json.loads(args.metadata.read_text(encoding="utf-8-sig"))
    else:
        command = [
            args.cargo, "metadata", "--locked", "--offline", "--format-version", "1",
            "--filter-platform", TARGET,
        ]
        metadata = json.loads(subprocess.check_output(command, cwd=ROOT, text=True, encoding="utf-8"))
    toolchain = args.rust_toolchain
    if toolchain is None:
        toolchain = pathlib.Path(subprocess.check_output(
            [args.rustc, "--print", "sysroot"], text=True, encoding="utf-8"
        ).strip())
    manifest, files = generate(metadata, args.fetch_missing, toolchain)
    # Assemble and validate every package before writing any output.
    mismatches = []
    for relative, data in files.items():
        path = within(ROOT, relative)
        if args.check:
            if not path.exists() or path.read_bytes() != data:
                mismatches.append(relative)
        else:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
    if mismatches:
        raise ValueError("Notices need regeneration: " + ", ".join(mismatches))
    verb = "Verified reproducible" if args.check else "Generated"
    print(f"{verb} notices for {len(manifest['packages'])} Windows Cargo packages; {len(files) - 2} text files.")


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError) as error:
        sys.exit(f"License notice error: {error}")
