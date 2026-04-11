from pathlib import Path

from pydantic import Field
from pydantic_settings import BaseSettings


def _default_database_url() -> str:
    db_path = Path("~/.local/share/hikyaku/registry.db").expanduser()
    return f"sqlite+aiosqlite:///{db_path}"


class Settings(BaseSettings):
    database_url: str = Field(
        default_factory=_default_database_url,
        validation_alias="HIKYAKU_DATABASE_URL",
    )
    broker_host: str = "0.0.0.0"
    broker_port: int = 8000
    broker_base_url: str = "http://localhost:8000"
    auth0_domain: str = ""
    auth0_client_id: str = ""
    auth0_audience: str = ""

    model_config = {"env_prefix": "", "populate_by_name": True}


settings = Settings()
