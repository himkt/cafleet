DROP TABLE asset_installs;

CREATE TABLE asset_installs (
    coding_agent TEXT NOT NULL,
    path TEXT NOT NULL,
    cafleet_version TEXT NOT NULL,
    installed_at TEXT NOT NULL,
    PRIMARY KEY (coding_agent, path)
);
