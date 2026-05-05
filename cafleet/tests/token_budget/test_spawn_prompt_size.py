"""Member spawn-prompt size budget (design doc 0000049, Surface 6 + 13).

The slim spawn prompt is one of the dominant per-spawn token costs (every
``cafleet member create`` pays it once). The Surface 6 target is ≤ 70
tokens after substitution; converting at ~6 chars / token gives a
generous ~420-character budget that flags any reversion to multi-paragraph
prose.
"""

from cafleet.cli import _MEMBER_PROMPT_TEMPLATE

_SAMPLE_SESSION_ID = "11111111-1111-1111-1111-111111111111"
_SAMPLE_AGENT_ID = "22222222-2222-2222-2222-222222222222"
_SAMPLE_DIRECTOR_ID = "33333333-3333-3333-3333-333333333333"


def _materialize() -> str:
    return _MEMBER_PROMPT_TEMPLATE.format(
        session_id=_SAMPLE_SESSION_ID,
        agent_id=_SAMPLE_AGENT_ID,
        director_agent_id=_SAMPLE_DIRECTOR_ID,
    )


def test_spawn_prompt_under_420_chars():
    """Hard char budget. 420 chars ≈ 70 tokens at the Anthropic / GPT-style
    chars-per-token ratio. A regression that re-introduces a multi-paragraph
    bootstrap blurb will overshoot immediately."""
    materialized = _materialize()
    assert len(materialized) <= 420, (
        f"spawn prompt grew to {len(materialized)} chars (budget 420). "
        f"current text:\n{materialized}"
    )


def test_spawn_prompt_template_under_300_chars_unsubstituted():
    """The template (with literal ``{session_id}`` etc. placeholders) is
    ALSO bounded. Substitution adds ~140 chars (three full UUIDs minus the
    placeholder names); the unsubstituted form should sit comfortably
    below 300 chars."""
    assert len(_MEMBER_PROMPT_TEMPLATE) <= 300, (
        f"spawn prompt template grew to {len(_MEMBER_PROMPT_TEMPLATE)} chars "
        f"(budget 300)"
    )


def test_spawn_prompt_substitutes_three_placeholders():
    """Sanity guard: the template MUST still expose ``{session_id}``,
    ``{agent_id}``, and ``{director_agent_id}`` so ``cli._resolve_prompt``
    has a target for each ``str.format`` kwarg. A regression that drops
    any of them silently is what this test catches."""
    materialized = _materialize()
    assert _SAMPLE_SESSION_ID in materialized
    assert _SAMPLE_AGENT_ID in materialized
    assert _SAMPLE_DIRECTOR_ID in materialized


def test_spawn_prompt_at_most_4_lines():
    """Slim prompt = ≤ 4 newline-separated lines after substitution. Surface
    6's target was 2 lines; 4 leaves headroom for a future single 1-line
    addition without the test going stale."""
    materialized = _materialize()
    line_count = materialized.count("\n") + 1
    assert line_count <= 4, f"spawn prompt grew to {line_count} lines (budget 4)"
