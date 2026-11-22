#!/usr/bin/env python3
"""Generate parity goldens with official Python `tiktoken`.

Maintainer-only script. CI never runs Python. Requires:

    python3 -m venv .venv
    .venv/bin/pip install tiktoken==0.12.0
    .venv/bin/python packages/atlas-tokenizer/scripts/generate_golden.py

`tiktoken==0.12.0` corresponds to upstream commit
`3591ff175d6a80efbe4fcc7f0e219ddd4b8c52f1`, the commit pinned by
`src/assets.rs`. Regenerate goldens whenever that pin changes.
"""

from __future__ import annotations

import base64
import json
from pathlib import Path

import tiktoken

ENCODINGS = [
    "r50k_base",
    "p50k_base",
    "p50k_edit",
    "cl100k_base",
    "o200k_base",
    "o200k_harmony",
]

SPECIALS = {
    "r50k_base": ["<|endoftext|>"],
    "p50k_base": ["<|endoftext|>"],
    "p50k_edit": [
        "<|endoftext|>",
        "<|fim_prefix|>",
        "<|fim_middle|>",
        "<|fim_suffix|>",
    ],
    "cl100k_base": [
        "<|endoftext|>",
        "<|fim_prefix|>",
        "<|fim_middle|>",
        "<|fim_suffix|>",
        "<|endofprompt|>",
    ],
    "o200k_base": ["<|endoftext|>", "<|endofprompt|>"],
    "o200k_harmony": [
        "<|startoftext|>",
        "<|endoftext|>",
        "<|return|>",
        "<|constrain|>",
        "<|channel|>",
        "<|start|>",
        "<|end|>",
        "<|message|>",
        "<|call|>",
        "<|reserved_200018|>",
        "<|reserved_201087|>",
    ],
}

CORPUS = [
    "",
    "a",
    "hello",
    "The quick brown fox jumps over the lazy dog. " * 40,
    "café naïve résumé",
    "中文分词测试",
    "مرحبا بالعالم",
    "नमस्ते दुनिया",
    "naive\u0301 combining",
    "👩‍💻 🧑🏽‍🚀 🏳️‍🌈",
    "spaces    and\ttabs",
    "line one\r\nline two\nline three",
    "lone\rcarriage",
    "trailing spaces   ",
    "   leading spaces",
    "don't can't won't I'll we're they've",
    "curly ’quotes’ and ‘apostrophes’",
    "1 12 123 1234 12345 123456",
    "fn main() {\n    println!(\"hello\");\n}\n",
    "def f(x):\n    return {'k': x + 1}\n",
    '{"a": 1, "b": [true, null, "x"]}\n',
    "SELECT id, name FROM users WHERE id = 42;\n",
    "# Heading\n\n- item\n- **bold** `code`\n",
    "repeat " * 512,
    "".join(chr(codepoint) for codepoint in range(256)),
    "<|endoftext|>",
    "<|fim_prefix|><|fim_middle|><|fim_suffix|>",
    "<|endofprompt|>",
]


def main() -> None:
    output = Path(__file__).resolve().parents[1] / "tests" / "golden"
    output.mkdir(parents=True, exist_ok=True)
    for name in ENCODINGS:
        encoding = tiktoken.get_encoding(name)
        rows = []
        for text in CORPUS:
            tokens = encoding.encode_ordinary(text)
            rows.append(
                {
                    "text": text,
                    "tokens": tokens,
                    "bytes": base64.b64encode(text.encode("utf-8")).decode("ascii"),
                    "special": False,
                }
            )
        for literal in SPECIALS[name]:
            tokens = encoding.encode(literal, allowed_special="all")
            rows.append(
                {
                    "text": literal,
                    "tokens": tokens,
                    "bytes": base64.b64encode(literal.encode("utf-8")).decode("ascii"),
                    "special": True,
                }
            )
        path = output / f"{name}.jsonl"
        with path.open("w", encoding="utf-8", newline="\n") as handle:
            for row in rows:
                handle.write(json.dumps(row, ensure_ascii=False, separators=(",", ":")))
                handle.write("\n")
        print(f"wrote {path} ({len(rows)} cases)")


if __name__ == "__main__":
    main()
