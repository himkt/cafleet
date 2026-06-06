"""Per-poll envelope size budget.

The compact rendered envelope (``output.format_json`` + projected
``output.render_task``) MUST stay materially smaller than the compact full
(un-projected) envelope. This file applies two complementary criteria:
fixture-anchored **UTF-8 byte** budgets (absolute caps on the rendered
envelope) and a **ratio guard** (compact slim stays below a fraction of
compact full). Together they detect ALL classes of regression — extra
fields, longer keys, removed truncation, multi-byte content sneaking in, etc.

Tokenizer choice: tests assert UTF-8 byte counts (cheap, deterministic).
The on-wire cost is bytes, not Python codepoints — the Unicode ellipsis
``…`` is 1 codepoint but 3 UTF-8 bytes, so ``len(s)`` would under-count.
1 token ≈ 4 bytes for English text in the GPT-style tokenizers cafleet's
downstream coding agents use, so the byte budgets below convert at roughly
the same ratio.
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
    """Compact JSON of a 5-task render MUST fit in 750 UTF-8 bytes. The
    exact number was sized for the typed-column shape (id8 + from8 + ts +
    text + optional kind/origin); a regression that re-introduces full
    UUIDs, re-adds dropped keys, or smuggles in multi-byte characters
    (e.g. the ``…`` truncation suffix is 3 UTF-8 bytes) will overshoot it
    immediately."""
    fixture = _five_unicast_fixture()
    rendered = output.format_json([output.render_task(t) for t in fixture])
    rendered_bytes = len(rendered.encode("utf-8"))
    assert rendered_bytes <= 750, (
        f"compact envelope grew to {rendered_bytes} UTF-8 bytes (budget 750); "
        f"check render_task or format_json for added fields"
    )


def test_compact_slim_envelope_smaller_than_compact_full():
    """Compact rendered (slim ``render_task``) ≤ budget of compact rendered
    (full / un-projected ``render_task(full=True)``) for the SAME fixture,
    measured in UTF-8 bytes (the on-wire cost). The default wire format
    (compact + projected) is what an agent actually pays for; the compact +
    full form is the un-projected baseline."""
    fixture = _five_unicast_fixture()
    compact_slim = output.format_json(
        [output.render_task(t, full=False) for t in fixture]
    )
    compact_full = output.format_json(
        [output.render_task(t, full=True) for t in fixture]
    )
    compact_slim_bytes = len(compact_slim.encode("utf-8"))
    compact_full_bytes = len(compact_full.encode("utf-8"))
    ratio = compact_slim_bytes / compact_full_bytes
    assert ratio <= 0.40, (
        f"compact-slim / compact-full UTF-8 byte ratio rose to {ratio:.3f} "
        f"(budget ≤ 0.40); "
        f"compact_slim={compact_slim_bytes}B compact_full={compact_full_bytes}B"
    )


def test_full_envelope_keeps_all_typed_column_keys():
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
    """Per-task compact render stays below 150 UTF-8 bytes (typed-column
    rendering keeps id+from+ts+text only — even with origin/kind it should
    not blow this budget). Measured in UTF-8 bytes so multi-byte characters
    in ``text`` (e.g. the ``…`` truncation suffix) cost what they actually
    cost on the wire. Assertion runs on the LARGEST item so a single
    bloated task is enough to fail the test."""
    fixture = _five_unicast_fixture()
    sizes = [
        len(output.format_json(output.render_task(t)).encode("utf-8")) for t in fixture
    ]
    assert max(sizes) <= 150, (
        f"per-task compact render largest = {max(sizes)} UTF-8 bytes "
        f"(budget 150); sizes={sizes}"
    )
