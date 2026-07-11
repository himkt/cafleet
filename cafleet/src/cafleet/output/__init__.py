"""Output rendering and formatting shared by the CLI and WebUI."""

from cafleet.output.formatters import (
    format_fleet_create,
    format_indexed_list,
    format_member,
    format_member_detail,
    format_member_list,
    format_message,
    format_monitor_config,
    format_monitor_status,
)
from cafleet.output.render import (
    format_json,
    render_message,
    render_messages_in_result,
    strip_ansi,
    truncate_message_text,
    truncate_text,
)

__all__ = [
    "strip_ansi",
    "format_json",
    "truncate_text",
    "truncate_message_text",
    "render_message",
    "render_messages_in_result",
    "format_message",
    "format_indexed_list",
    "format_member_detail",
    "format_fleet_create",
    "format_member",
    "format_member_list",
    "format_monitor_status",
    "format_monitor_config",
]
