-- Synthetic counters produced by installed SessionDB 0.21.7+18.g590af66.
-- Source commit: 590af66a58c2b3eb3c5edd1dcfefa09c625ae18a. Only selected statistics columns; no messages, prompts, configuration or credentials.
CREATE TABLE schema_version(version INTEGER NOT NULL);
INSERT INTO schema_version VALUES(31);
CREATE TABLE sessions(id TEXT,input_tokens INTEGER,output_tokens INTEGER,cache_read_tokens INTEGER,cache_write_tokens INTEGER,api_call_count INTEGER,PRIMARY KEY(id));
INSERT INTO sessions VALUES('absolute-aux',500,50,0,0,5);
INSERT INTO sessions VALUES('main-aux',100,10,0,0,1);
INSERT INTO sessions VALUES('main-voice-aux',120,12,0,0,2);
CREATE TABLE session_model_usage(session_id TEXT,model TEXT,task TEXT,billing_provider TEXT,billing_base_url TEXT,billing_mode TEXT,input_tokens INTEGER,output_tokens INTEGER,cache_read_tokens INTEGER,cache_write_tokens INTEGER,api_call_count INTEGER,PRIMARY KEY(session_id,model,task,billing_provider,billing_base_url,billing_mode));
INSERT INTO session_model_usage VALUES('absolute-aux','aux-model','vision','','','',30,3,0,0,1);
INSERT INTO session_model_usage VALUES('main-aux','aux-model','vision','','','',30,3,0,0,1);
INSERT INTO session_model_usage VALUES('main-aux','main-model','','','','',100,10,0,0,1);
INSERT INTO session_model_usage VALUES('main-voice-aux','aux-model','vision','','','',30,3,0,0,1);
INSERT INTO session_model_usage VALUES('main-voice-aux','main-model','','','','',100,10,0,0,1);
INSERT INTO session_model_usage VALUES('main-voice-aux','voice-model','voice_chat','','','',20,2,0,0,1);
