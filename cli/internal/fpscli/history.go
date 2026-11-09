// SPDX-License-Identifier: GPL-3.0-only

package fpscli

import (
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"time"
)

// The REST API has no operation that lists stored configurations, so the
// client keeps a list of the configurations that it uploaded. The file lives
// below the user's state directory and holds no secret.

// historyEntry is one uploaded configuration.
type historyEntry struct {
	Controller string `json:"controller"`
	ID         string `json:"id"`
	Name       string `json:"name"`
	CreatedAt  string `json:"created_at,omitempty"`
	UploadedAt string `json:"uploaded_at"`
	Source     string `json:"source,omitempty"`
}

type historyFile struct {
	Configs []historyEntry `json:"configs"`
}

// historyPath returns the history file, or "" when no home directory exists.
func (a *app) historyPath() string {
	if a.g.HistoryFile != "" {
		return a.g.HistoryFile
	}
	if d := a.env.Getenv("XDG_STATE_HOME"); d != "" {
		return filepath.Join(d, "fps", "configs.json")
	}
	if h := a.env.Getenv("HOME"); h != "" {
		return filepath.Join(h, ".local", "state", "fps", "configs.json")
	}
	return ""
}

func (a *app) readHistory() (historyFile, error) {
	var h historyFile
	path := a.historyPath()
	if path == "" {
		return h, nil
	}
	data, err := os.ReadFile(path) //nolint:gosec // G304: the path comes from the user's settings
	if errors.Is(err, os.ErrNotExist) {
		return h, nil
	}
	if err != nil {
		return h, err
	}
	if err := json.Unmarshal(data, &h); err != nil {
		return historyFile{}, err
	}
	return h, nil
}

// recordUpload adds an uploaded configuration to the history. A failure only
// warns, because the upload itself succeeded.
func (a *app) recordUpload(ref ConfigRef, source string) {
	path := a.historyPath()
	if path == "" {
		return
	}
	h, err := a.readHistory()
	if err != nil {
		a.eprintf("fps: warning: the history file %s is unreadable, so it is not updated: %v\n", path, err)
		return
	}
	entry := historyEntry{
		Controller: a.g.URL,
		ID:         ref.ID,
		Name:       ref.Name,
		CreatedAt:  ref.CreatedAt,
		UploadedAt: a.env.Now().UTC().Format(time.RFC3339),
		Source:     source,
	}
	replaced := false
	for i, e := range h.Configs {
		if e.Controller == entry.Controller && e.ID == entry.ID {
			h.Configs[i], replaced = entry, true
		}
	}
	if !replaced {
		h.Configs = append(h.Configs, entry)
	}
	data, err := json.MarshalIndent(h, "", "  ")
	if err == nil {
		err = os.MkdirAll(filepath.Dir(path), 0o700)
	}
	if err == nil {
		err = os.WriteFile(path, append(data, '\n'), 0o600)
	}
	if err != nil {
		a.eprintf("fps: warning: cannot update the history file %s: %v\n", path, err)
	}
}
