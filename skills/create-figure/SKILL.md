---
name: create-figure
description: >
  Create data visualizations and charts using matplotlib. Triggered when user
  asks to create a chart, plot, graph, or visualize data. Also invokable via
  /create-figure. Do NOT use plt.show() — always save to PNG files.
---

# Create Figure

Generate matplotlib charts. Scripts, outputs, and data go in separate subdirectories under `figures/`.
Only the execution borrows a uv environment that has matplotlib installed (referenced as `$CLAUDE_HOME` below; substitute the absolute path of the user's Claude config directory).

**Before writing any script, read the Chart Type Selection and Color Rules sections.** All charts in a deck share the same `C_BAR` / `C_BAR_SEC` palette regardless of data topic.

## Procedure

### 0. Resolve directories

**CRITICAL — placeholder convention.** `${FIGURE_BASE}`, `${BASE}`, `${SRC_DIR}`, `${OUTPUT_DIR}`, and `${DATA_DIR}` in this document are **template placeholders, NOT shell environment variables**. You must mentally resolve each one to a concrete absolute path and write that literal path into the script file via the Write tool.

**Do NOT** run `export FIGURE_BASE=...`, `FIGURE_BASE=... uv run ...`, or any other shell variable assignment. Bash calls in Claude Code are ephemeral — values set in one call do not persist to the next. The placeholders are resolved entirely in your head, not in the shell.

**Resolve `${BASE}` in this order:**

1. **Calling-context override**: If a parent skill's spawn prompt told you the figure base directory (e.g., `/research-presentation` passes its research folder as the figure base), use that path literally as `${BASE}`. Skip base-dir resolution.
2. **Otherwise**: Load `Skill(base-dir)` and follow its procedure (no path argument; CWD-based inference applies). If the resolved `${BASE}` is the user's Claude config directory (i.e., not a real project root), override to `${BASE} = /tmp/claude-code`.

**Derive the subdirectories** (each is a literal path string you will embed in the script):

- `${SRC_DIR} = ${BASE}/figures/src`
- `${OUTPUT_DIR} = ${BASE}/figures/output`
- `${DATA_DIR} = ${BASE}/figures/data`

Example resolution: if the calling skill said "use `/tmp/claude-code/researches/foo` as the figure base", then `${SRC_DIR}` in your head is `/tmp/claude-code/researches/foo/figures/src` — that literal string is what you write into the Python script.

If the directories do not exist yet, the Write tool auto-creates parent directories when you write the script file — do NOT call `mkdir`.

All subsequent steps use `${SRC_DIR}`, `${OUTPUT_DIR}`, and `${DATA_DIR}` as literal resolved paths. Never create scripts or outputs inside the user's Claude config directory.

**Font:** No setup needed. The theme font `Noto Sans` is available as a system font. Scripts set `plt.rcParams['font.family'] = 'Noto Sans'` (see template below).

### 1. Create the script

Use the Write tool to create a `.py` file in `${SRC_DIR}`.

The script must follow this pattern. **Replace `${OUTPUT_DIR}` and `${DATA_DIR}` below with the literal concrete paths you resolved in Step 0** — the Python source you write must contain real path strings, not `${...}` syntax:

```python
import pathlib
import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt

# REPLACE these with literal paths from Step 0 resolution.
# e.g. pathlib.Path("/tmp/claude-code/researches/foo/figures/output")
OUTPUT_DIR = pathlib.Path("${OUTPUT_DIR}")
DATA_DIR = pathlib.Path("${DATA_DIR}")

plt.rcParams['font.family'] = 'Noto Sans'

# Data input
data_path = DATA_DIR / "data.csv"

# Create the figure
fig, ax = plt.subplots(constrained_layout=True)
# ... plotting logic ...

# Output — filename matches script name
script_stem = pathlib.Path(__file__).stem
output_path = OUTPUT_DIR / f"{script_stem}.png"
plt.savefig(output_path, dpi=150, bbox_inches="tight", facecolor='white')
plt.close()
print(f"Saved: {output_path}")
```

Key points:
- **All figure text in English** — matplotlib defaults don't support CJK; use English for labels/titles/legends
- **No `ax.set_title()`** — when embedded in slides, the slide heading is the title
- `matplotlib.use("Agg")` before `import matplotlib.pyplot`
- Output filename matches script name (`chart.py` → `chart.png`)
- Never `plt.show()`, always `plt.savefig()` then `plt.close()`
- PNG, `dpi=150`, `bbox_inches="tight"`, `facecolor='white'`

### 2. Execute the script

`$CLAUDE_HOME` (substitute with the absolute path of the user's Claude config directory) has a `pyproject.toml` that manages matplotlib via uv.
Run a single Bash call with `--frozen` and `--project` to use this environment without changing CWD:

```
uv run --frozen --project $CLAUDE_HOME ${SRC_DIR}/script_name.py
```

`--frozen` prevents lockfile updates. `--project $CLAUDE_HOME` uses the venv there without changing CWD.

### 3. Verify the result

Use the Read tool to load the output PNG from `${OUTPUT_DIR}` and show it to the user.

## Data handling

CSV:
```python
import csv
with open(DATA_DIR / "sales.csv") as f:
    rows = list(csv.DictReader(f))
```

JSON:
```python
import json
with open(DATA_DIR / "metrics.json") as f:
    data = json.load(f)
```

Inline data (from web search or user input):
```python
categories = ["Q1", "Q2", "Q3", "Q4"]
values = [120, 185, 240, 310]
```

## Chart Type Selection

Pick the right visualization for the data. Do NOT default to bar charts for everything.

| Data pattern | Chart type | matplotlib |
|---|---|---|
| Category comparison, ranking | Horizontal bar | `ax.barh()` |
| Time series, trend | Line chart | `ax.plot()` — format dates with `mdates.DateFormatter` (see below) |
| Correlation between 2 variables | Scatter plot | `ax.scatter()` |
| Distribution of one variable | Histogram | `ax.hist()` |
| Distribution comparison across groups | Box plot or violin plot | `ax.boxplot()` / `ax.violinplot()` |
| Matrix, cross-tabulation | Heatmap | `ax.imshow()` + annotate |
| Part-of-whole composition | Stacked bar | `ax.bar(bottom=...)` |
| Before/after, paired comparison | Dumbbell chart | `ax.hlines()` + `ax.scatter()` |
| Time-based categories (quarters, years) | Vertical bar | `ax.bar()` |

**Horizontal vs vertical bars**: Prefer `barh` when category labels are text. Use vertical `bar` only for time-based x-axes.

### Time series date axes

```python
import matplotlib.dates as mdates
ax.xaxis.set_major_formatter(mdates.DateFormatter('%Y-%m'))
ax.xaxis.set_major_locator(mdates.MonthLocator(interval=3))
fig.autofmt_xdate()  # auto-rotate date labels
```

### Multi-panel layout

For side-by-side comparison panels, use `subplot_mosaic` or `GridSpec`. **All panels must share the same Y-axis range** (see Prohibited section).

```python
# Named panels — cleaner than index-based access
fig, axes = plt.subplot_mosaic(
    [['left', 'right']],
    figsize=(12, 5), constrained_layout=True,
)
axes['left'].barh(categories, values_a, color=C_BAR)
axes['right'].barh(categories, values_b, color=C_BAR_SEC)

# Complex layouts with GridSpec
from matplotlib.gridspec import GridSpec
fig = plt.figure(figsize=(12, 8), constrained_layout=True)
gs = GridSpec(2, 3, figure=fig)
ax_wide = fig.add_subplot(gs[0, :])   # top row, full width
ax_left = fig.add_subplot(gs[1, :2])  # bottom row, 2/3 width
ax_right = fig.add_subplot(gs[1, 2])  # bottom row, 1/3 width
```

## Color Rules

**1 chart = max 2 colors.** This is the single most important design rule. Violating it produces amateurish, noisy charts.

### Allowed palette

These approximate the Slidev theme's CSS tokens (defined in OKLCH inside the my-slidev skill's theme stylesheet). When updating colors, keep both in sync.

```python
# Primary (use for most bars/lines)
C_BAR = '#3B82F6'         # blue-500

# Muted secondary (use for secondary series, negative values, or contrast)
C_BAR_SEC = '#94A3B8'     # slate-400

# Accent (use for ONE highlighted item only — never for multiple bars)
C_ACCENT = '#1E40AF'      # blue-800 (darker shade of primary)

# Negative accent (use only when one specific item is the "worst")
C_NEGATIVE = '#DC2626'    # red-600

# Spine / grid
C_TEXT = '#1E293B'
C_TEXT_SEC = '#64748B'
C_GRID = '#E2E8F0'
```

### Decision table

| Data pattern | Colors to use |
|---|---|
| Single series, all same type | `C_BAR` for all |
| Single series, grouping by category | Lightness steps of blue (`#1E40AF` / `#3B82F6` / `#93C5FD`) |
| Two series (e.g., Verified vs Pro) | `C_BAR` + `C_BAR_SEC` |
| Positive vs negative values | `C_BAR` (positive) + `C_BAR_SEC` (negative) |
| Highlight one worst item | `C_BAR_SEC` for all + `C_NEGATIVE` for worst |
| Highlight one best/top item | `C_BAR` for all + `C_ACCENT` (darker) for top |

### Annotations for highlighting

Use annotations to call out a specific data point instead of (or alongside) color highlighting:

```python
ax.annotate('Peak', xy=(x_peak, y_peak),
            xytext=(x_peak + offset, y_peak + offset),
            fontsize=10, color=C_TEXT,
            arrowprops=dict(arrowstyle='->', color=C_TEXT_SEC, lw=1.5))
```

Keep annotation text short (1–3 words). Use `C_TEXT` for text, `C_TEXT_SEC` for arrows.

### Prohibited

- **Semantic coloring** — do NOT choose colors based on what the data "means." A vulnerability chart uses `C_BAR` (blue), not red. An adoption chart uses `C_BAR`, not green. Data meaning comes from axis labels and slide context. All charts in a deck must look like they belong together
- **3+ hues in one chart** — never blue + red + green + orange. Use `C_NEGATIVE` or `C_ACCENT` for at most 1 highlighted item; everything else is `C_BAR` or `C_BAR_SEC`
- **`C_NEGATIVE` / `C_ACCENT` as primary color** — highlight colors are only for emphasizing a single item, never for all data points
- **Different Y-axis scales in small multiples** — side-by-side panels MUST share the same Y-axis range

### Standard axes styling

Apply to every figure:

```python
ax.spines['top'].set_visible(False)
ax.spines['right'].set_visible(False)
ax.spines['left'].set_color(C_GRID)
ax.spines['bottom'].set_color(C_GRID)
ax.tick_params(colors=C_TEXT_SEC)
ax.yaxis.grid(True, alpha=0.3, color='#CBD5E1')
ax.set_axisbelow(True)
```

Always use `facecolor='white'` in `plt.savefig()`.

### Legend positioning

When a legend is needed, place it outside the plot area to avoid obscuring data:

```python
ax.legend(bbox_to_anchor=(1.05, 1), loc='upper left', frameon=False)
```

For horizontal legends below the chart (useful in multi-series line plots):

```python
ax.legend(bbox_to_anchor=(0.5, -0.12), loc='upper center', ncol=3, frameon=False)
```

Use `frameon=False` to keep the look clean. Limit legend entries to ≤ 5; if more, reconsider the chart design.

## Out of scope

- Libraries beyond matplotlib (no seaborn, plotly, etc.)
- Interactive display via `plt.show()`
- GUI or web-based visualization
- Automated data analysis or statistical inference
