// SPDX-License-Identifier: GPL-3.0-only

// Package captures starts packet captures on the agents of a run and serves
// the pcapng files. It implements the three capture operations of the REST
// contract: POST and GET /v1/runs/{id}/captures and GET
// /v1/runs/{id}/captures/{capture_id}.
//
// A capture starts with StartCapture on every agent of the run, under one
// controller chosen capture id, and the service records its state per run. An
// agent ends a capture at its byte limit or at the end of the run, and the
// service ends one that still runs when somebody downloads it. The download
// then fetches the file of every agent with the fetch_capture command of CON-13
// and the UploadCapture call of the agent manager, stores each file below the
// capture directory, and serves the files of all agents as one pcapng stream.
// A transfer that breaks resumes with a new transfer id at the offset that the
// service holds, and a file counts only when its size and its SHA-256 digest
// equal what the agent reported at the end of the capture.
//
// The capture directory holds one directory per run and capture, with a
// meta.json that records the state, and one file per agent. It has mode 0700,
// because a capture of TLS sessions holds the keys that decrypt them.
package captures
