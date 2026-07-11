"""Tests for ``output.format_member_detail`` — the ``member show`` text formatter.

Extended per design 0000116: the ``--full`` block gains ``kind``, ``skills``
(compact JSON array or ``-`` when empty), and a placement sub-block that
appears only when a placement row exists (``placement:   none`` otherwise);
``None`` fields inside the placement render ``-``.
"""

import pytest

from cafleet import output


def _member(**overrides) -> dict:
    base = {
        "member_id": 2,
        "name": "Director",
        "description": "Root Director for this fleet",
        "status": "active",
        "registered_at": "2026-04-15T10:00:00+00:00",
        "kind": "director",
        "skills": [],
        "placement": {
            "mux_session": "main",
            "mux_window_id": "@3",
            "mux_pane_id": "%0",
            "coding_agent": "claude",
            "created_at": "2026-04-15T10:00:00+00:00",
        },
    }
    base.update(overrides)
    return base


def test_format_member_detail_default__compact_one_line_row():
    assert output.format_member_detail(_member()) == "2 Director active"


def test_format_member_detail_full__placed_block_exact_layout():
    rendered = output.format_member_detail(_member(), full=True)
    assert rendered == "\n".join(
        [
            "  member_id:   2",
            "  name:        Director",
            "  description: Root Director for this fleet",
            "  status:      active",
            "  kind:        director",
            "  skills:      -",
            "  placement:",
            "    backend:    claude",
            "    session:    main",
            "    window_id:  @3",
            "    pane_id:    %0",
            "    created_at: 2026-04-15T10:00:00+00:00",
        ]
    )


def test_format_member_detail_full__placementless_block_exact_layout():
    rendered = output.format_member_detail(
        _member(
            member_id=3,
            name="standalone",
            description="Placementless member",
            kind="member",
            placement=None,
        ),
        full=True,
    )
    assert rendered == "\n".join(
        [
            "  member_id:   3",
            "  name:        standalone",
            "  description: Placementless member",
            "  status:      active",
            "  kind:        member",
            "  skills:      -",
            "  placement:   none",
        ]
    )


def test_format_member_detail_full__member_kind_and_pane():
    rendered = output.format_member_detail(
        _member(
            member_id=5,
            name="alice",
            kind="member",
            placement={
                "mux_session": "main",
                "mux_window_id": "@3",
                "mux_pane_id": "%7",
                "coding_agent": "claude",
                "created_at": "2026-04-15T10:02:00+00:00",
            },
        ),
        full=True,
    )
    assert "  kind:        member" in rendered
    assert "    pane_id:    %7" in rendered


def test_format_member_detail_full__pending_pane_id_renders_dash():
    placement = _member()["placement"]
    placement["mux_pane_id"] = None
    rendered = output.format_member_detail(_member(placement=placement), full=True)
    assert "    pane_id:    -" in rendered


def test_format_member_detail_full__skills_render_compact_json_array():
    rendered = output.format_member_detail(
        _member(skills=[{"id": "echo"}]),
        full=True,
    )
    assert '  skills:      [{"id":"echo"}]' in rendered


@pytest.mark.parametrize(
    ("description", "expected"),
    [
        ("x" * 200, "x" * 60 + "…"),
        ("concise", "concise"),
    ],
)
def test_format_member_detail_full__description_truncated_at_60(description, expected):
    rendered = output.format_member_detail(_member(description=description), full=True)
    assert f"  description: {expected}" in rendered
