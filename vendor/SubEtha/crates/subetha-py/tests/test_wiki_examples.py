"""Every Python program the SubEtha Guide shows, run as a reader runs it.

A ```python block that names a file (```python {filename="producer.py"})
is a program. A page's Python programs run in page order from one fresh
folder, with the temporary directory pointed at it as well, each as a
process of its own under the interpreter running these tests, so they
import the package built for this run. Each runs to its end before the
next starts unless it is marked alongside="true", and none runs longer
than DEADLINE seconds. A program marked for one operating system
(os="linux") runs there only. A program passes when it exits
successfully and prints what the Output block after it shows.

The blocks are read by the rules crates/subetha-wiki-tests/src/page.rs
reads them by, which checks every other block of the Guide.
"""

import os
import pathlib
import re
import subprocess
import sys
import tempfile

import pytest

DEADLINE = 60

CONTENT = pathlib.Path(__file__).resolve().parents[3] / "wiki" / "content"

# Rust's names for the operating systems a block can be marked for.
HOST_OS = (
    "windows" if sys.platform == "win32"
    else "macos" if sys.platform == "darwin"
    else "freebsd" if sys.platform.startswith("freebsd")
    else sys.platform
)


def parse_info(info):
    """A fence's info string as its language and its attributes."""
    head, brace, rest = info.partition("{")
    lang = re.split(r"[,\s]", head.strip(), maxsplit=1)[0]
    attrs = {}
    if brace:
        inner = rest.rstrip()
        if not inner.endswith("}"):
            raise ValueError(f"attributes {{{rest} are not one {{...}} list")
        for key, quoted, bare in re.findall(r'(\w+)=(?:"([^"]*)"|([^\s,]*))', inner[:-1]):
            attrs[key] = quoted if quoted else bare
    return lang, attrs


def blocks(text):
    """Every fenced block of a page with its line, language, attributes,
    body, tab group, tab, and the group that closed just before it."""
    lines = text.splitlines()
    out = []
    group = None
    groups = 0
    tab = None
    closed = None
    i = 0
    while i < len(lines):
        line = lines[i]
        stripped = line.lstrip()
        if stripped.startswith("```"):
            indent = len(line) - len(stripped)
            lang, attrs = parse_info(stripped[3:])
            body = []
            j = i + 1
            while True:
                if j >= len(lines):
                    raise ValueError(f"line {i + 1}: a fence that never closes")
                if lines[j].strip() == "```":
                    break
                inner = lines[j]
                cut = min(len(inner) - len(inner.lstrip()), indent)
                body.append(inner[cut:])
                j += 1
            out.append({
                "line": i + 1,
                "lang": lang,
                "attrs": attrs,
                "body": "".join(b + "\n" for b in body),
                "group": group,
                "tab": tab,
                "after_group": closed if group is None else None,
            })
            closed = None
            i = j + 1
            continue
        shortcode = stripped.rstrip()
        if shortcode.startswith("{{< tabs"):
            group = groups
            groups += 1
        elif shortcode == "{{< /tabs >}}":
            closed, group, tab = group, None, None
        elif shortcode.startswith("{{< tab "):
            m = re.search(r'name="([^"]*)"', shortcode)
            if not m:
                raise ValueError(f"line {i + 1}: a tab with no name")
            tab = m.group(1)
        elif shortcode == "{{< /tab >}}":
            tab = None
        i += 1
    return out


LANGS = {"rust": "Rust", "python": "Python", "powershell": "PowerShell", "c": "C"}


def programs(page, found):
    """The page's programs, each with the Output block that belongs to it."""
    progs = []
    last_in_tab = {}
    last_outside = None
    for block in found:
        attrs = block["attrs"]
        if block["lang"] == "text" and attrs.get("filename") == "Output":
            if block["group"] is not None and block["tab"] is not None:
                key = (block["group"], block["tab"])
                owners = [last_in_tab.pop(key)] if key in last_in_tab else []
            elif block["after_group"] is not None:
                g = block["after_group"]
                owners = [p for (lg, _), p in last_in_tab.items() if lg == g]
                last_in_tab = {k: p for k, p in last_in_tab.items() if k[0] != g}
            else:
                owners = [last_outside] if last_outside is not None else []
                last_outside = None
            if not owners:
                raise ValueError(f"{page}:{block['line']}: an Output block with no program before it")
            for p in owners:
                progs[p]["output"] = (block["line"], block["body"])
            continue
        surface = LANGS.get(block["lang"])
        if surface is None or "filename" not in attrs:
            continue
        if block["tab"] is not None and block["tab"] != surface:
            raise ValueError(f"{page}:{block['line']}: a {surface} program in the {block['tab']} tab")
        progs.append({
            "page": page,
            "line": block["line"],
            "surface": surface,
            "filename": attrs["filename"],
            "os": attrs.get("os"),
            "alongside": attrs.get("alongside") == "true",
            "source": block["body"],
            "output": None,
        })
        index = len(progs) - 1
        if block["group"] is not None and block["tab"] is not None:
            last_in_tab[(block["group"], block["tab"])] = index
        else:
            last_outside = index
    return progs


def normalized(text):
    """Text with line ends made \\n, trailing whitespace and trailing blank
    lines dropped, as a page's Output block and a program's output are
    compared."""
    lines = [line.rstrip() for line in text.splitlines()]
    while lines and not lines[-1]:
        lines.pop()
    return "\n".join(lines)


def pages_with_python():
    found = []
    for path in sorted(CONTENT.rglob("*.md")):
        page = path.relative_to(CONTENT).as_posix()
        progs = [p for p in programs(page, blocks(path.read_text(encoding="utf-8"))) if p["surface"] == "Python"]
        if progs:
            found.append(pytest.param(page, progs, id=page))
    return found


PAGES = pages_with_python()


def test_the_guide_holds_python_programs():
    assert PAGES, f"no page under {CONTENT} shows a Python program"


def start(program, folder):
    target = folder / program["filename"]
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(program["source"], encoding="utf-8")
    env = dict(os.environ, TEMP=str(folder), TMP=str(folder), TMPDIR=str(folder))
    if sys.platform != "win32":
        env["XDG_RUNTIME_DIR"] = str(folder)
    return subprocess.Popen(
        [sys.executable, program["filename"]],
        cwd=folder,
        env=env,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )


def judge(program, process):
    """What went wrong with a finished or overdue program, or None."""
    at = f"{program['page']}:{program['line']} ({program['filename']})"
    try:
        out, err = process.communicate(timeout=DEADLINE)
        overdue = False
    except subprocess.TimeoutExpired:
        process.kill()
        out, err = process.communicate()
        overdue = True
    printed = normalized(out.decode("utf-8", "replace"))
    detail = f"\n--- printed\n{printed}\n--- standard error\n{err.decode('utf-8', 'replace').rstrip()}"
    if overdue:
        return f"{at}: still running after {DEADLINE} s, so stopped{detail}"
    if process.returncode != 0:
        return f"{at}: exited with {process.returncode}{detail}"
    if program["output"] is not None:
        line, shown = program["output"]
        shown = normalized(shown)
        if printed != shown:
            return f"{at}: printed other than the Output block at line {line} shows:\n--- shown\n{shown}{detail}"
    return None


@pytest.mark.parametrize("page, progs", PAGES)
def test_every_python_program_on_the_page_runs_and_prints_what_it_shows(page, progs):
    failures = []
    skipped = []
    with tempfile.TemporaryDirectory(prefix="subetha-wiki-") as scratch:
        folder = pathlib.Path(scratch)
        alongside = []
        for program in progs:
            if program["os"] is not None and program["os"] != HOST_OS:
                skipped.append(f"{program['page']}:{program['line']} ({program['os']})")
                continue
            process = start(program, folder)
            if program["alongside"]:
                alongside.append((program, process))
                continue
            failures.append(judge(program, process))
        for program, process in alongside:
            failures.append(judge(program, process))
    failures = [f for f in failures if f is not None]
    for s in skipped:
        print(f"for another operating system: {s}")
    assert not failures, "\n\n".join(failures)
