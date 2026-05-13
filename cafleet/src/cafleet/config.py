from pathlib import Path

from pydantic import Field
from pydantic_settings import BaseSettings


def _default_database_url() -> str:
    db_path = Path("~/.local/share/cafleet/registry.db").expanduser()
    return f"sqlite:///{db_path}"


class Settings(BaseSettings):
    database_url: str = Field(
        default_factory=_default_database_url,
        validation_alias="CAFLEET_DATABASE_URL",
    )
    broker_host: str = Field(
        default="127.0.0.1",
        validation_alias="CAFLEET_BROKER_HOST",
    )
    broker_port: int = Field(
        default=8000,
        validation_alias="CAFLEET_BROKER_PORT",
    )
    max_text_len: int = Field(
        default=200,
        validation_alias="CAFLEET_MAX_TEXT_LEN",
    )


settings = Settings()
