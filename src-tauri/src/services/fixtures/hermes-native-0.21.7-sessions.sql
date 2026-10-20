-- Synthetic rows in the SessionDB schema-31 shape of Hermes 0.21.7+18.g590af66
-- (source commit 590af66a58c2b3eb3c5edd1dcfefa09c625ae18a). Only the columns Ofox
-- reads, plus message body columns that must never leave SQLite.
-- T0 = 1790820000 = 2026-10-01T02:00:00Z.
CREATE TABLE schema_version(version INTEGER NOT NULL);
INSERT INTO schema_version VALUES(31);
CREATE TABLE sessions(
    id TEXT PRIMARY KEY,
    source TEXT NOT NULL,
    model TEXT,
    title TEXT,
    cwd TEXT,
    parent_session_id TEXT,
    started_at REAL NOT NULL,
    ended_at REAL,
    last_activity_at REAL,
    message_count INTEGER DEFAULT 0,
    input_tokens INTEGER DEFAULT 0,
    output_tokens INTEGER DEFAULT 0,
    cache_read_tokens INTEGER DEFAULT 0,
    cache_write_tokens INTEGER DEFAULT 0,
    reasoning_tokens INTEGER DEFAULT 0,
    api_call_count INTEGER DEFAULT 0
);
CREATE TABLE session_model_usage(
    session_id TEXT NOT NULL,
    model TEXT NOT NULL,
    billing_provider TEXT NOT NULL DEFAULT '',
    billing_base_url TEXT NOT NULL DEFAULT '',
    billing_mode TEXT NOT NULL DEFAULT '',
    task TEXT NOT NULL DEFAULT '',
    api_call_count INTEGER NOT NULL DEFAULT 0,
    input_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    cache_read_tokens INTEGER NOT NULL DEFAULT 0,
    cache_write_tokens INTEGER NOT NULL DEFAULT 0,
    reasoning_tokens INTEGER NOT NULL DEFAULT 0,
    first_seen REAL,
    last_seen REAL,
    PRIMARY KEY (session_id, model, billing_provider, billing_base_url, billing_mode, task)
);
CREATE TABLE messages(
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL,
    role TEXT NOT NULL,
    content TEXT,
    tool_call_id TEXT,
    tool_calls TEXT,
    tool_name TEXT,
    timestamp REAL NOT NULL,
    token_count INTEGER,
    active INTEGER NOT NULL DEFAULT 1,
    compacted INTEGER NOT NULL DEFAULT 0,
    display_metadata TEXT
);

-- Main loop plus an auxiliary vision call, both on the Ofox route.
INSERT INTO sessions VALUES('main-aux','cli','main-model','Alpha','/work/alpha',NULL,1790820000.0,1790820060.0,1790820050.0,4,100,10,5,2,0,1);
INSERT INTO session_model_usage VALUES('main-aux','main-model','custom','https://api.ofox.ai/v1?key=SECRET','','',1,100,10,5,2,0,1790820001.0,1790820040.0);
INSERT INTO session_model_usage VALUES('main-aux','aux-model','custom','https://api.ofox.ai/v1?key=SECRET','','vision',1,30,3,0,0,0,1790820045.0,1790820045.0);
INSERT INTO messages(session_id,role,content,tool_calls,timestamp) VALUES('main-aux','user','private body',NULL,1790820001.0);
INSERT INTO messages(session_id,role,content,tool_calls,timestamp) VALUES('main-aux','assistant','private reply','[{"function":{"name":"terminal"}}]',1790820004.0);
INSERT INTO messages(session_id,role,content,tool_call_id,tool_name,timestamp) VALUES('main-aux','tool','private result','call-1','terminal',1790820006.0);
INSERT INTO messages(session_id,role,content,timestamp) VALUES('main-aux','assistant','private done',1790820009.0);
-- A compacted copy of the previous turn collapses into its first id.
INSERT INTO messages(session_id,role,content,timestamp,active,compacted) VALUES('main-aux','assistant','private done',1790820009.0,0,1);
INSERT INTO messages(session_id,role,content,timestamp,display_metadata) VALUES('main-aux','user','private scaffold',1790820010.0,'{"model_only":true}');
INSERT INTO messages(session_id,role,content,timestamp,active) VALUES('main-aux','user','private stale',1790820011.0,0);

-- Voice shares the session totals; vision is added on top of them.
INSERT INTO sessions VALUES('main-voice-aux','cli','main-model',NULL,'/work/beta',NULL,1790820100.0,NULL,NULL,0,120,12,0,0,0,3);
INSERT INTO session_model_usage VALUES('main-voice-aux','main-model','','','','',1,100,10,0,0,0,1790820105.0,1790820110.0);
INSERT INTO session_model_usage VALUES('main-voice-aux','voice-model','','','','voice_chat',1,20,2,0,0,0,1790820111.0,1790820111.0);
INSERT INTO session_model_usage VALUES('main-voice-aux','aux-model','','','','vision',1,30,3,0,0,0,1790820112.0,1790820112.0);

-- Totals from before session_model_usage existed: no model rows at all.
INSERT INTO sessions VALUES('legacy','cli','old-model','Legacy',NULL,NULL,1790823600.0,1790823700.0,NULL,0,50,5,0,0,0,1);

-- Model rows explain only part of the totals.
INSERT INTO sessions VALUES('partial','cli','main-model','Partial','/work/alpha',NULL,1790820200.0,NULL,1790820300.0,0,80,8,0,0,0,2);
INSERT INTO session_model_usage VALUES('partial','main-model','','','','',1,60,6,0,0,0,1790820240.0,1790820250.0);

-- A task this Hermes version does not document.
INSERT INTO sessions VALUES('unknown-task','cli','main-model','Unknown task',NULL,NULL,1790820400.0,NULL,NULL,0,10,1,0,0,0,2);
INSERT INTO session_model_usage VALUES('unknown-task','main-model','','','','',1,10,1,0,0,0,1790820410.0,1790820410.0);
INSERT INTO session_model_usage VALUES('unknown-task','future-model','','','','future_task',1,7,1,0,0,0,1790820420.0,1790820420.0);

-- No last_seen: the row falls back to the session's latest time.
INSERT INTO sessions VALUES('null-seen','cli','main-model','Null seen',NULL,NULL,1790820500.0,1790820520.0,NULL,1,5,1,0,0,0,1);
INSERT INTO session_model_usage VALUES('null-seen','main-model','','','','',1,5,1,0,0,0,NULL,NULL);
INSERT INTO messages(session_id,role,content,timestamp) VALUES('null-seen','assistant','private late',1790820530.0);

-- 23:53 to 00:10 in UTC+8: the row lands on its last_seen day.
INSERT INTO sessions VALUES('cross-day','cli','main-model','Cross day',NULL,NULL,1790870000.0,NULL,1790871000.0,0,9,1,0,0,0,1);
INSERT INTO session_model_usage VALUES('cross-day','main-model','','','','',1,9,1,0,0,0,1790870340.0,1790871000.0);

-- A model row whose session is gone.
INSERT INTO session_model_usage VALUES('ghost','ghost-model','','','','',1,4,4,0,0,0,1790820600.0,1790820600.0);
