"""Read a saved official Nous Portal models page and emit validated JSON facts."""

import argparse
from collections import Counter
from decimal import Decimal, ROUND_HALF_UP
import hashlib
from html.parser import HTMLParser
import json
from pathlib import Path
import re
import sys


SOURCE_URL = "https://portal.nousresearch.com/models"
DISCOUNT_FRACTION_ABSOLUTE_TOLERANCE = Decimal("1e-12")
PRICE_FIELDS = (
    "promptDollarsPerMillion",
    "completionDollarsPerMillion",
    "originalPromptDollarsPerMillion",
    "originalCompletionDollarsPerMillion",
)
MODEL_FIELDS = {
    "id", "displayName", "providerKey", "providerLabel", "modality",
    "isFree", "hasDiscount", "promptDiscountPercent", "minDiscountFraction",
    "isEmbeddingsModel", "usageRank", "isNousHosted", *PRICE_FIELDS,
}
BLOCK_ELEMENTS = {
    "article", "br", "button", "div", "footer", "h1", "h2", "h3", "header",
    "li", "main", "nav", "p", "section", "table", "td", "th", "tr",
}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, f"Duplicate JSON field: {key}")
        result[key] = value
    return result


def reject_constant(value):
    raise ValueError(f"Nonfinite JSON number: {value}")


DECODER = json.JSONDecoder(
    parse_float=Decimal,
    parse_constant=reject_constant,
    object_pairs_hook=unique_object,
)


class SavedPage(HTMLParser):
    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.hidden = None
        self.script_parts = []
        self.scripts = []
        self.visible_parts = []

    def handle_starttag(self, tag, attrs):
        if tag in {"script", "style"}:
            require(self.hidden is None, "Nested script/style element")
            self.hidden = tag
            self.script_parts = []
        elif self.hidden is None and tag in BLOCK_ELEMENTS:
            self.visible_parts.append(" ")

    def handle_endtag(self, tag):
        if tag in {"script", "style"}:
            require(self.hidden == tag, "Unbalanced script/style element")
            if tag == "script":
                self.scripts.append("".join(self.script_parts))
            self.hidden = None
        elif self.hidden is None and tag in BLOCK_ELEMENTS:
            self.visible_parts.append(" ")

    def handle_data(self, data):
        if self.hidden == "script":
            self.script_parts.append(data)
        elif self.hidden is None:
            self.visible_parts.append(data)


def flight_stream(scripts):
    parts = []
    for script in scripts:
        for match in re.finditer(r"self\.__next_f\.push\(\s*", script):
            packet, end = DECODER.raw_decode(script, match.end())
            require(script[end:].lstrip().startswith(")"), "Flight packet has a non-JSON argument")
            require(isinstance(packet, list) and packet and type(packet[0]) is int, "Invalid Flight packet")
            if packet[0] == 1:
                require(len(packet) == 2 and isinstance(packet[1], str), "Flight channel 1 requires a JSON string")
                parts.append(packet[1])
            else:
                require(packet == [0] or packet == [2, None], "Unreviewed Flight channel")
    require(parts, "No Flight string payloads found")
    return "".join(parts), len(parts)


def browser_rows(stream):
    matches = list(re.finditer(r'(?<!\\)"rows"\s*:\s*', stream))
    require(len(matches) == 1, "Expected exactly one complete browser rows property")
    rows, _ = DECODER.raw_decode(stream, matches[0].end())
    require(isinstance(rows, list) and rows, "Browser rows must be a nonempty array")
    return rows


def visible_numbers(visible, pattern, name):
    matches = {tuple(map(int, match.groups())) for match in re.finditer(pattern, visible)}
    require(len(matches) == 1, f"Missing or conflicting visible {name}")
    return next(iter(matches))


def reconcile_pagination(visible, models):
    text_count, = visible_numbers(visible, r"\bText\s*\(\s*(\d+)\s*\)", "Text count")
    embedding_count, = visible_numbers(visible, r"\bEmbeddings\s*\(\s*(\d+)\s*\)", "Embeddings count")
    start, end, total = visible_numbers(
        visible, r"\bShowing\s+(\d+)\s*[-–]\s*(\d+)\s+of\s+(\d+)\s+models\b", "page range"
    )
    page, pages = visible_numbers(visible, r"\bPage\s+(\d+)\s+of\s+(\d+)\b", "page count")
    require(total == len(models) == text_count + embedding_count, "Visible total differs from the complete rows array")
    require(1 <= start <= end <= total and 1 <= page <= pages, "Invalid visible pagination bounds")
    if page < pages:
        page_size = end - start + 1
    elif page == 1:
        page_size = total
    else:
        require((start - 1) % (page - 1) == 0, "Final page has an inconsistent start")
        page_size = (start - 1) // (page - 1)
    require(page_size > 0, "Invalid display page size")
    require(start == (page - 1) * page_size + 1, "Visible page start disagrees with its index")
    require(end == min(page * page_size, total), "Visible page end disagrees with its index")
    require(pages == (total + page_size - 1) // page_size, "Visible page count does not cover the total")
    counts = {"text": text_count, "embeddings": embedding_count}
    require(Counter(row["modality"] for row in models) == counts, "Visible type counts disagree with model modalities")
    return {
        "visible_type_counts": counts,
        "visible_page_range": [start, end, total],
        "visible_page_count": pages,
        "unique_id_count": total,
        "selected_text_count": text_count,
        "excluded_embedding_count": embedding_count,
    }


def number(value, name):
    require(type(value) is int or isinstance(value, Decimal), f"{name} requires a JSON number")
    result = Decimal(value)
    require(result.is_finite() and result >= 0, f"{name} must be finite and nonnegative")
    return result


def decimal_text(value, name, price=False):
    if value is None:
        return None
    numeric = number(value, name)
    return "0.00" if price and numeric == 0 else format(numeric, "f")


def validate_model(row, index):
    require(isinstance(row, dict) and set(row) == MODEL_FIELDS, f"Model {index} has missing or unreviewed fields")
    for name in ("id", "displayName", "providerKey", "providerLabel"):
        require(isinstance(row[name], str) and row[name].strip(), f"Model {index} lacks {name}")
    model_id = row["id"]
    require(model_id == model_id.strip() and not any(character.isspace() or ord(character) < 32 or ord(character) == 127 for character in model_id), "Ambiguous model ID")
    require(isinstance(row["modality"], str) and row["modality"] in {"text", "embeddings"}, f"Unknown modality for {model_id}")
    for name in ("isFree", "hasDiscount", "isEmbeddingsModel", "isNousHosted"):
        require(type(row[name]) is bool, f"{name} must be boolean for {model_id}")
    require(row["isEmbeddingsModel"] == (row["modality"] == "embeddings"), f"Conflicting model type for {model_id}")
    billed = [number(row[name], name) for name in PRICE_FIELDS[:2]]
    original = [None if row[name] is None else number(row[name], name) for name in PRICE_FIELDS[2:]]
    require(row["isFree"] == all(value == 0 for value in billed), f"Free flag conflicts with billed rates for {model_id}")
    if row["hasDiscount"]:
        require(all(value is not None for value in original), f"Missing discount list rates for {model_id}")
        require(all(current < listed for current, listed in zip(billed, original)), f"Discount conflicts with list rates for {model_id}")
        percent = row["promptDiscountPercent"]
        require(type(percent) is int and 0 < percent <= 100, f"Missing or invalid discount percentage for {model_id}")
        fraction = number(row["minDiscountFraction"], "minDiscountFraction")
        require(0 < fraction <= 1, f"Invalid discount fraction for {model_id}")
        reductions = [(listed - current) / listed for current, listed in zip(billed, original)]
        rounded_input_percent = (reductions[0] * 100).to_integral_value(rounding=ROUND_HALF_UP)
        require(percent == rounded_input_percent, f"Input discount percentage conflicts with billed/list rates for {model_id}")
        require(abs(fraction - min(reductions)) <= DISCOUNT_FRACTION_ABSOLUTE_TOLERANCE, f"Minimum discount fraction conflicts with billed/list rates for {model_id}")
    else:
        require(all(value is None for value in original), f"List rates require reviewed discount semantics for {model_id}")
        require(row["promptDiscountPercent"] is None and row["minDiscountFraction"] is None, f"Discount qualifiers conflict with flag for {model_id}")
    rank = row["usageRank"]
    require(rank is None or type(rank) is int and rank >= 0, f"Invalid usage rank for {model_id}")
    normalized = {"source_index": index, **row}
    for name in PRICE_FIELDS:
        normalized[name] = decimal_text(row[name], name, price=True)
    normalized["minDiscountFraction"] = decimal_text(row["minDiscountFraction"], "minDiscountFraction")
    return normalized


def extract(source_bytes):
    page = SavedPage()
    page.feed(source_bytes.decode("utf-8"))
    page.close()
    require(page.hidden is None, "Truncated script/style element")
    stream, fragments = flight_stream(page.scripts)
    rows = browser_rows(stream)
    models = [validate_model(row, index) for index, row in enumerate(rows)]
    require(len({row["id"] for row in models}) == len(models), "Duplicate or conflicting exact model IDs")
    metadata = reconcile_pagination("".join(page.visible_parts), models)
    return {
        "metadata": {
            "source_url": SOURCE_URL,
            "source_html_sha256": hashlib.sha256(source_bytes).hexdigest(),
            "flight_string_parts": fragments,
            **metadata,
            "discount_end_dates": "Not published",
            "time_or_context_price_tiers": "Not published",
        },
        "models": models,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("saved_html", type=Path)
    args = parser.parse_args()
    try:
        facts = extract(args.saved_html.read_bytes())
        output = json.dumps(facts, ensure_ascii=False, allow_nan=False, indent=2)
    except (OSError, UnicodeError, ValueError) as exc:
        print(f"Portal extraction failed: {exc}", file=sys.stderr)
        return 1
    print(output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
