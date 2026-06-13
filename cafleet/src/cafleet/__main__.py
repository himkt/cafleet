"""``python -m cafleet`` entry point.

Delegates to the click ``cli`` group so the detached monitor worker can be
re-exec'd as ``[sys.executable, "-m", "cafleet", …]`` — using ``sys.executable``
guarantees the child runs in the same environment (and ``cafleet`` install) as
the launching CLI.
"""

from cafleet.cli import cli

if __name__ == "__main__":
    cli()
