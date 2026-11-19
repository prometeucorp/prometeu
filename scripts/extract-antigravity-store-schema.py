#!/usr/bin/env python3
"""Extract field metadata from an explicitly named AGY binary, never user stores.

The installed Go binary embeds FileDescriptorProto records. This helper records
selected functional field metadata for the independent bounded Rust decoder.
It does not copy provider implementation code.
"""
import argparse
import hashlib
import json
import re
from pathlib import Path

LIMIT = 16 * 1024 * 1024
SELECTED = {
    "gemini_coder.Step": [1, 4, 5, 19, 20, 30, 31, 114, 140],
    "exa.cortex_pb.CortexErrorDetails": [1, 2],
    "exa.cortex_pb.CortexStepMetadata": [3, 4, 9, 24, 29],
    "exa.cortex_pb.CortexStepUserInput": [1, 2, 19],
    "exa.cortex_pb.CortexStepPlannerResponse": [1, 3, 7, 8, 12],
    "exa.cortex_pb.CortexStepCheckpoint": [4, 5, 6, 8, 16, 17, 29],
    "exa.cortex_pb.CortexStepSystemMessage": [1, 2, 3, 4, 5, 6],
    "exa.cortex_pb.StepRenderInfo": [1, 2, 4, 5, 6, 7, 10, 11],
    "exa.cortex_pb.StepRenderInfo.ContentBlock": [1, 2, 3, 4, 5],
    "exa.cortex_pb.AgentMessage": [1, 2, 3, 4, 5, 6, 9, 11],
    "exa.cortex_pb.CortexStepSystemMessage.MetadataEntry": [1, 2],
    "exa.codeium_common_pb.ChatToolCall": [1, 2, 3],
    "exa.cortex_pb.CortexStepGeneric": [1, 2],
    "exa.cortex_pb.GenericStepResult": [1, 6, 7, 10, 11],
    "exa.cortex.core.tools.RunCommand": [1, 2, 5, 6, 7, 8, 9],
    "exa.codeium_common_pb.ModelUsageStats": [2, 3, 4, 5, 9],
    "exa.codeium_common_pb.ModelInfo": [8],
}


def varint(data, offset):
    value = 0
    for index in range(10):
        byte = data[offset]
        offset += 1
        if index == 9 and byte > 1:
            raise ValueError("varint overflow")
        value |= (byte & 127) << (7 * index)
        if not byte & 128:
            return value, offset
    raise ValueError("unterminated varint")


def field(data, offset):
    tag, offset = varint(data, offset)
    number, wire = tag >> 3, tag & 7
    if not 0 < number <= 0x1FFFFFFF:
        raise ValueError("invalid field")
    if wire == 0:
        value, offset = varint(data, offset)
    else:
        if wire == 2:
            size, offset = varint(data, offset)
        elif wire in (1, 5):
            size = 8 if wire == 1 else 4
        else:
            raise ValueError("unsupported wire kind")
        if size > LIMIT or offset + size > len(data):
            raise ValueError("truncated or excessive field")
        value = data[offset:offset + size]
        offset += size
    return (number, wire, value), offset


def fields(data):
    result, offset = [], 0
    while offset < len(data):
        if len(result) >= 65536:
            raise ValueError("excessive field count")
        item, offset = field(data, offset)
        result.append(item)
    return result


def one(data, number, default=None):
    return next((value for key, _, value in fields(data) if key == number), default)


def text(value):
    return value.decode("utf-8") if value else ""


def messages(data, prefix, schemas, depth=0):
    if depth > 32:
        raise ValueError("descriptor nesting limit")
    name = prefix + "." + text(one(data, 1))
    result = []
    for number, wire, value in fields(data):
        if wire != 2:
            continue
        if number == 2:
            result.append({
                "name": text(one(value, 1)), "number": one(value, 3),
                "label": one(value, 4), "type": one(value, 5),
                "type_name": text(one(value, 6)),
            })
        elif number == 3:
            messages(value, name, schemas, depth + 1)
    schemas[name] = result


def extract(binary):
    schemas = {}
    pattern = rb"(?:third_party|google)/[A-Za-z0-9_./-]{3,180}\.proto"
    for match in re.finditer(pattern, binary):
        for prefix in range(2, 5):
            start = match.start() - prefix
            try:
                if start < 0 or binary[start] != 10:
                    continue
                length, after = varint(binary, start + 1)
                if after != match.start() or length != len(match.group()):
                    continue
                offset, seen, chunk = start, set(), []
                while offset - start < LIMIT:
                    try:
                        item, next_offset = field(binary, offset)
                    except (IndexError, ValueError):
                        break
                    number, wire, _ = item
                    if number not in range(1, 15) or wire not in (0, 2):
                        break
                    if number in (1, 2, 8, 9, 12, 14) and number in seen:
                        break
                    seen.add(number)
                    chunk.append(item)
                    offset = next_offset
                package = next((text(v) for n, w, v in chunk if n == 2 and w == 2), "")
                if not any(part in package for part in ("cortex", "gemini_coder", "codeium_common")):
                    continue
                for number, wire, value in chunk:
                    if number == 4 and wire == 2:
                        messages(value, package, schemas)
            except (IndexError, UnicodeError, ValueError):
                # Filename-like bytes elsewhere in the binary are not descriptors.
                continue
    return schemas


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--cli-version", required=True)
    args = parser.parse_args()
    binary = args.binary.read_bytes()
    schemas = extract(binary)
    missing = set(SELECTED) - schemas.keys()
    if missing:
        raise SystemExit("Required descriptors missing: " + ", ".join(sorted(missing)))
    result = {
        "cliVersion": args.cli_version,
        "binarySha256": hashlib.sha256(binary).hexdigest(),
        "messages": {name: [item for item in schemas[name] if item["number"] in numbers]
                     for name, numbers in SELECTED.items()},
    }
    args.output.write_text(json.dumps(result, indent=2) + "\n")


if __name__ == "__main__":
    main()
