CREATE TABLE master_log
(
    id  BIGINT PRIMARY KEY NOT NULL,
    ts  BIGINT             NOT NULL,
    raw BLOB               NOT NULL
);

CREATE TABLE replica_log
(
    id      BIGINT PRIMARY KEY NOT NULL,
    ts      BIGINT             NOT NULL,
    recv_ts BIGINT             NOT NULL,
    replay  BOOLEAN            NOT NULL,
    raw     BLOB               NOT NULL
);