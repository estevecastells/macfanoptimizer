#!/usr/bin/env python3
"""Render the README comparison chart (light + dark SVG) from the simulator.

    cargo build --release -p fanctl && python3 docs/images/make_chart.py

A work session (idle → 8 min of heavy work → idle) as two stacked panels on a
shared time axis: chip temperature and fan speed. Two panels, never one chart
with two y-axes. Static SVG because GitHub READMEs strip scripts. Palette: the
first three categorical slots of the validated dataviz reference palette
(all-pairs safe in both modes); the light-mode aqua is below 3:1 contrast, so
every line also carries a visible direct label, plus a legend.
"""

import csv
import io
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
FANCTL = ROOT / "target" / "release" / "fanctl"
OUT = Path(__file__).resolve().parent
DURATION_S = 900
WORK = (120, 600)  # seconds of heavy load, from the simulator's `session` scenario

# (label, fanctl simulate args, palette slot)
SERIES = [
    ("MacFanOptimizer (Smart)", ["--mode", "smart", "--profile", "balanced"], 0),
    ("macOS default (emulated)", ["--mode", "system"], 1),
    ("Fixed 6000 rpm", ["--mode", "fixed", "--rpm", "6000"], 2),
]

THEMES = {
    "light": {
        "surface": "#fcfcfb", "text": "#0b0b0b", "muted": "#52514e", "grid": "#e6e5e1", "band": "#f1f0ec",
        "series": ["#2a78d6", "#eb6834", "#1baf7a"],
    },
    "dark": {
        "surface": "#1a1a19", "text": "#ffffff", "muted": "#c3c2b7", "grid": "#33332f", "band": "#242422",
        "series": ["#3987e5", "#d95926", "#199e70"],
    },
}

W = 760
LEFT, RIGHT = 56, 190
FONT = "-apple-system, BlinkMacSystemFont, 'Segoe UI', Helvetica, Arial, sans-serif"

# Two panels stacked: (title, field, y range, ticks, unit formatter, top, height)
PANELS = [
    ("Chip temperature", "die_c", (30, 100), range(30, 101, 10), lambda v: f"{v:.0f}°", 112, 190),
    ("Fan speed", "fan_rpm", (0, 8000), range(0, 8001, 2000), lambda v: f"{v / 1000:.0f}k" if v else "off", 352, 120),
]
H = 520


def simulate(args):
    out = subprocess.run(
        [str(FANCTL), "simulate", "session", "--duration", str(DURATION_S), "--csv", *args],
        check=True, capture_output=True, text=True,
    ).stdout
    return [{k: float(r[k]) for k in ("t", "die_c", "fan_rpm")} for r in csv.DictReader(io.StringIO(out))]


def x(t):
    return LEFT + (W - LEFT - RIGHT) * t / DURATION_S


def panel_y(v, lo, hi, top, height):
    return top + height * (1 - (v - lo) / (hi - lo))


def esc(s):
    return s.replace("&", "&amp;").replace("<", "&lt;")


def place_labels(wanted, min_gap):
    """Nudge label y positions apart (top to bottom) so they never collide."""
    out = []
    for y in sorted(wanted):
        if out and y - out[-1] < min_gap:
            y = out[-1] + min_gap
        out.append(y)
    return out


def render(theme, data):
    th = THEMES[theme]
    peaks = {name: max(p["die_c"] for p in pts) for name, pts, _ in data}
    p = [
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" width="{W}" height="{H}" '
        f'role="img" aria-labelledby="t d" font-family="{FONT}">',
        '<title id="t">Chip temperature and fan speed during a work session (simulated)</title>',
        '<desc id="d">'
        + esc("; ".join(f"{n}: peaks at {peaks[n]:.0f} °C" for n, _, _ in data))
        + ". Smart and macOS keep the fans off while idle; the fixed setting runs them at 6000 rpm throughout.</desc>",
        f'<rect width="{W}" height="{H}" rx="12" fill="{th["surface"]}"/>',
        f'<text x="{LEFT}" y="30" font-size="15" font-weight="600" fill="{th["text"]}">'
        "Cooler under load, silent when idle</text>",
        f'<text x="{LEFT}" y="49" font-size="12" fill="{th["muted"]}">'
        "A work session in the thermal simulator calibrated on an M5 Pro MacBook Pro: "
        "idle, 8 min of heavy work (~75 W), idle</text>",
    ]
    # Legend row.
    lx = LEFT
    for name, _, slot in data:
        p.append(f'<line x1="{lx}" x2="{lx + 18}" y1="74" y2="74" stroke="{th["series"][slot]}" '
                 'stroke-width="3" stroke-linecap="round"/>')
        p.append(f'<text x="{lx + 24}" y="78" font-size="12" fill="{th["text"]}">{esc(name)}</text>')
        lx += 24 + 7.0 * len(name) + 24

    for title, field, (lo, hi), ticks, fmt, top, height in PANELS:
        # Shaded "heavy work" band, labelled once on the top panel.
        p.append(f'<rect x="{x(WORK[0]):.1f}" y="{top}" width="{x(WORK[1]) - x(WORK[0]):.1f}" '
                 f'height="{height}" fill="{th["band"]}"/>')
        if top == PANELS[0][5]:
            # Bottom of the band: the lines are high up during the work, so this space is empty.
            p.append(f'<text x="{(x(WORK[0]) + x(WORK[1])) / 2:.1f}" y="{top + height - 10}" font-size="11" '
                     f'text-anchor="middle" fill="{th["muted"]}">heavy work</text>')
        p.append(f'<text x="{LEFT}" y="{top - 10}" font-size="12" font-weight="600" fill="{th["text"]}">{title}</text>')
        for v in ticks:
            gy = panel_y(v, lo, hi, top, height)
            p.append(f'<line x1="{LEFT}" x2="{W - RIGHT}" y1="{gy:.1f}" y2="{gy:.1f}" stroke="{th["grid"]}"/>')
            p.append(f'<text x="{LEFT - 8}" y="{gy + 4:.1f}" font-size="11" text-anchor="end" '
                     f'fill="{th["muted"]}">{fmt(v)}</text>')
        # Lines, ours drawn last so it sits on top.
        for name, pts, slot in reversed(data):
            d = "M" + " L".join(f"{x(q['t']):.1f},{panel_y(q[field], lo, hi, top, height):.1f}" for q in pts)
            p.append(f'<path d="{d}" fill="none" stroke="{th["series"][slot]}" stroke-width="2" '
                     'stroke-linejoin="round" stroke-linecap="round"/>')

        # Direct labels in the right margin: the value during heavy work.
        mid = (WORK[0] + WORK[1]) / 2 + 120
        rows = []
        for name, pts, slot in data:
            v = min(pts, key=lambda q: abs(q["t"] - mid))[field]
            rows.append((panel_y(v, lo, hi, top, height), v, name, slot))
        rows.sort()
        gap = 30 if field == "die_c" else 28
        for (wy, v, name, slot), ly in zip(rows, place_labels([r[0] for r in rows], gap)):
            ly = min(ly, top + height + 6)
            value = f"{v:.0f} °C" if field == "die_c" else ("off" if v < 1 else f"{v:,.0f} rpm")
            p.append(f'<circle cx="{W - RIGHT + 12}" cy="{ly - 4:.1f}" r="4" fill="{th["series"][slot]}"/>')
            p.append(f'<text x="{W - RIGHT + 22}" y="{ly:.1f}" font-size="12" font-weight="600" '
                     f'fill="{th["text"]}">{value}</text>')
            p.append(f'<text x="{W - RIGHT + 22}" y="{ly + 14:.1f}" font-size="11" fill="{th["muted"]}">'
                     f'{esc(name.split(" (")[0])}</text>')

    # Shared time axis under the bottom panel.
    _, _, _, _, _, top, height = PANELS[-1]
    for m in range(0, DURATION_S // 60 + 1, 5):
        p.append(f'<text x="{x(m * 60):.1f}" y="{top + height + 20}" font-size="11" text-anchor="middle" '
                 f'fill="{th["muted"]}">{m} min</text>')
    p.append(f'<text x="{W - RIGHT + 12}" y="{PANELS[0][5] - 10}" font-size="11" fill="{th["muted"]}">'
             "during work</text>")
    p.append("</svg>")
    return "\n".join(p) + "\n"


def main():
    data = [(name, simulate(args), slot) for name, args, slot in SERIES]
    for theme in THEMES:
        path = OUT / f"chart-session-{theme}.svg"
        path.write_text(render(theme, data))
        print(f"wrote {path.relative_to(ROOT)}")
    for name, pts, _ in data:
        print(f"  {name}: peak {max(q['die_c'] for q in pts):.1f} °C")


if __name__ == "__main__":
    main()
