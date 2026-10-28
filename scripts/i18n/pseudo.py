#!/usr/bin/env python3
"""Build an accented, expanded test catalog without altering placeholders."""
import argparse
import json
import re
from pathlib import Path


def entries(text):
    for block in text.split("\n\n"):
        fields, current = {}, None
        for line in block.splitlines():
            match = re.match(r'(msgid_plural|msgid|msgstr(?:\[\d+\])?)\s+(".*")$', line)
            if match:
                current = match[1]
                fields[current] = json.loads(match[2])
            elif line.startswith('"') and current:
                fields[current] += json.loads(line)
        if "msgid" in fields:
            yield fields


def pseudo(message):
    if message in ("Quickshare", "GNOME Quickshare") or message.startswith("%"):
        return message
    accents = str.maketrans("AaEeIiOoUuCcNn", "ÁáÉéÍíÓóÚúÇçÑñ")
    parts = re.split(r'(\{[a-zA-Z_]+\})', message)
    expanded = "".join(part if re.fullmatch(r'\{[a-zA-Z_]+\}', part) else part.translate(accents) for part in parts)
    return "⟦" + expanded.rstrip(";") + " ···⟧" + (";" if message.endswith(";") else "")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("template", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    quote = lambda value: json.dumps(value, ensure_ascii=False)
    header = ("Project-Id-Version: gnome-quickshare 0.1.0\n"
              "PO-Revision-Date: 2026-10-28 00:00+0000\n"
              "Last-Translator: Automated layout check\n"
              "Language-Team: Test catalog\nLanguage: zz\n"
              "MIME-Version: 1.0\nContent-Type: text/plain; charset=UTF-8\n"
              "Content-Transfer-Encoding: 8bit\nPlural-Forms: nplurals=2; plural=(n != 1);\n")
    output = ['msgid ""\nmsgstr ' + quote(header)]
    for entry in entries(args.template.read_text()):
        message = entry["msgid"]
        if not message:
            continue
        block = []
        if re.search(r'\{[a-zA-Z_]+\}', message):
            block.append("#, rust-format")
        block.append("msgid " + quote(message))
        if "msgid_plural" in entry:
            block.extend(["msgid_plural " + quote(entry["msgid_plural"]),
                          "msgstr[0] " + quote(pseudo(message)),
                          "msgstr[1] " + quote(pseudo(entry["msgid_plural"]))])
        else:
            block.append("msgstr " + quote(pseudo(message)))
        output.append("\n".join(block))
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text("\n\n".join(output) + "\n")


if __name__ == "__main__":
    main()
