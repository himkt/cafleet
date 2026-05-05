"""Per-poll envelope size budget (design doc 0000049, Surface 1 + 13).

The compact rendered envelope (default ``output.format_json(..., pretty=False)``
+ ``output.render_task``) MUST stay materially smaller than the legacy
indented envelope. The pass criterion below is a fixture-anchored character
budget rather than a derived percentage so the test detects ALL classes of
regression — extra fields, longer keys, removed truncation, etc.

Tokenizer choice: tests assert character counts (cheap, deterministic).
A real-tokenizer cross-check belongs in a separate helper (Step 15
implementation note). 1 token ≈ 4 characters for English text in the
GPT-style tokenizers cafleet's downstream coding agents use, so the
character budgets below convert at roughly the same ratio.
"""

from cafleet import output


def _five_unicast_fixture() -> list[dict]:
    """Five typed-column unicast tasks. Stable UUIDs so the byte counts are
    reproducible across runs and tokenizers."""
    return [
        {
            "task_id": "11111111-1111-1111-1111-111111111111",
            "context_id": "22222222-2222-2222-2222-222222222222",
            "from_agent_id": "33333333-3333-3333-3333-333333333333",
            "to_agent_id": "22222222-2222-2222-2222-222222222222",
            "type": "unicast",
            "created_at": "2026-05-05T10:00:00.000000+00:00",
            "status_state": "input_required",
            "status_timestamp": "2026-05-05T10:00:00.000000+00:00",
            "origin_task_id": None,
            "text": "first message body",
        },
        {
            "task_id": "44444444-4444-4444-4444-444444444444",
            "context_id": "22222222-2222-2222-2222-222222222222",
            "from_agent_id": "33333333-3333-3333-3333-333333333333",
            "to_agent_id": "22222222-2222-2222-2222-222222222222",
            "type": "unicast",
            "created_at": "2026-05-05T10:00:01.000000+00:00",
            "status_state": "input_required",
            "status_timestamp": "2026-05-05T10:00:01.000000+00:00",
            "origin_task_id": None,
            "text": "second message body",
        },
        {
            "task_id": "55555555-5555-5555-5555-555555555555",
            "context_id": "22222222-2222-2222-2222-222222222222",
            "from_agent_id": "66666666-6666-6666-6666-666666666666",
            "to_agent_id": "22222222-2222-2222-2222-222222222222",
            "type": "unicast",
            "created_at": "2026-05-05T10:00:02.000000+00:00",
            "status_state": "completed",
            "status_timestamp": "2026-05-05T10:00:02.000000+00:00",
            "origin_task_id": None,
            "text": "third message body",
        },
        {
            "task_id": "77777777-7777-7777-7777-777777777777",
            "context_id": "22222222-2222-2222-2222-222222222222",
            "from_agent_id": "33333333-3333-3333-3333-333333333333",
            "to_agent_id": "22222222-2222-2222-2222-222222222222",
            "type": "unicast",
            "created_at": "2026-05-05T10:00:03.000000+00:00",
            "status_state": "input_required",
            "status_timestamp": "2026-05-05T10:00:03.000000+00:00",
            "origin_task_id": "88888888-8888-8888-8888-888888888888",
            "text": "fourth message body",
        },
        {
            "task_id": "99999999-9999-9999-9999-999999999999",
            "context_id": "22222222-2222-2222-2222-222222222222",
            "from_agent_id": "33333333-3333-3333-3333-333333333333",
            "to_agent_id": "22222222-2222-2222-2222-222222222222",
            "type": "unicast",
            "created_at": "2026-05-05T10:00:04.000000+00:00",
            "status_state": "input_required",
            "status_timestamp": "2026-05-05T10:00:04.000000+00:00",
            "origin_task_id": None,
            "text": "fifth message body",
        },
    ]


def test_compact_envelope_fits_within_byte_budget():
    """Compact JSON of a 5-task render MUST fit in 750 bytes. The exact
    number was sized for the typed-column shape (id8 + from8 + ts + text +
    optional kind/origin); a regression that re-introduces full UUIDs or
    re-adds dropped keys will overshoot it immediately."""
    fixture = _five_unicast_fixture()
    rendered = output.format_json(
        [output.render_task(t) for t in fixture],
        pretty=False,
    )
    assert len(rendered) <= 750, (
        f"compact envelope grew to {len(rendered)} bytes (budget 750); "
        f"check render_task or format_json for added fields"
    )


def test_compact_slim_envelope_at_most_30pct_of_pretty_full():
    """Compact rendered (slim ``render_task``) ≤ 30 % of pretty rendered
    (full / un-projected ``render_task(full=True)``) for the SAME fixture.
    This is the cumulative-savings story from the design doc: the default
    wire format (compact + projected) is what an agent actually pays for,
    and the pretty + full form is what the legacy envelope cost."""
    fixture = _five_unicast_fixture()
    compact_slim = output.format_json(
        [output.render_task(t, full=False) for t in fixture],
        pretty=False,
    )
    pretty_full = output.format_json(
        [output.render_task(t, full=True) for t in fixture],
        pretty=True,
    )
    ratio = len(compact_slim) / len(pretty_full)
    assert ratio <= 0.30, (
        f"compact-slim / pretty-full ratio rose to {ratio:.3f} (budget ≤ 0.30); "
        f"compact_slim={len(compact_slim)} pretty_full={len(pretty_full)}"
    )


def test_full_envelope_keeps_legacy_keys():
    """``render_task(task, full=True)`` MUST keep every typed-column key so
    operators using ``--full`` still get the verbose shape they reach for
    when debugging. This is a guardrail against an over-eager projection
    that drops fields from the ``full`` branch by accident."""
    fixture = _five_unicast_fixture()
    rendered = [output.render_task(t, full=True) for t in fixture]

    expected_keys = {
        "task_id",
        "context_id",
        "from_agent_id",
        "to_agent_id",
        "type",
        "created_at",
        "status_state",
        "status_timestamp",
        "origin_task_id",
        "text",
    }
    for task in rendered:
        assert expected_keys.issubset(task.keys())


def test_compact_envelope_per_task_below_150_bytes():
    """Per-task compact render stays below 150 bytes (typed-column rendering
    keeps id+from+ts+text only — even with origin/kind it should not blow
    this budget). Assertion runs on the LARGEST item so a single bloated
    task is enough to fail the test."""
    fixture = _five_unicast_fixture()
    sizes = [
        len(output.format_json(output.render_task(t), pretty=False))
        for t in fixture
    ]
    assert max(sizes) <= 150, (
        f"per-task compact render largest = {max(sizes)} bytes (budget 150); "
        f"sizes={sizes}"
    )
