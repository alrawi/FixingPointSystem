// SPDX-License-Identifier: GPL-3.0-only

package compiler

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"

	"github.com/santhosh-tekuri/jsonschema/v6"
)

// ErrUnknownModule reports module settings for a module that the catalog
// lacks or that the profile mix does not use.
var ErrUnknownModule = errors.New("unknown module")

// moduleSettings validates the modules section of the configuration against
// the settings schema of each module and returns the settings of the modules
// that the plan uses, by module name. A module whose manifest has no settings
// schema accepts no settings.
func moduleSettings(cfg Config, catalog []Scenario, used map[string]ModuleRef) (map[string]map[string]any, error) {
	schemas := make(map[string]json.RawMessage, len(catalog))
	for _, sc := range catalog {
		if len(schemas[sc.Module.Name]) == 0 {
			schemas[sc.Module.Name] = sc.SettingsSchema
		}
	}
	out := map[string]map[string]any{}
	for _, name := range sortedKeys(cfg.Modules) {
		path := "modules." + name + ".settings"
		settings := cfg.Modules[name].Settings
		schema, known := schemas[name]
		if !known {
			return nil, fmt.Errorf("modules.%s: %w: no installed module has this name", name, ErrUnknownModule)
		}
		if _, ok := used[name]; !ok {
			return nil, fmt.Errorf("modules.%s: %w: no scenario of the profile mix comes from this module", name, ErrUnknownModule)
		}
		if len(settings) == 0 {
			continue
		}
		if len(schema) == 0 {
			return nil, invalid(path, "module %s accepts no settings", name)
		}
		if err := validateSettings(path, schema, settings); err != nil {
			return nil, err
		}
		out[name] = settings
	}
	return out, nil
}

// validateSettings checks settings against one JSON Schema document.
func validateSettings(path string, schema json.RawMessage, settings map[string]any) error {
	doc, err := jsonschema.UnmarshalJSON(bytes.NewReader(schema))
	if err != nil {
		return fmt.Errorf("%w: %s: settings schema is not JSON: %w", ErrInvalidCatalog, path, err)
	}
	c := jsonschema.NewCompiler()
	const url = "settings.schema.json"
	if err := c.AddResource(url, doc); err != nil {
		return fmt.Errorf("%w: %s: settings schema: %w", ErrInvalidCatalog, path, err)
	}
	compiled, err := c.Compile(url)
	if err != nil {
		return fmt.Errorf("%w: %s: settings schema: %w", ErrInvalidCatalog, path, err)
	}
	raw, err := json.Marshal(settings)
	if err != nil {
		return invalid(path, "settings are not JSON values: %v", err)
	}
	inst, err := jsonschema.UnmarshalJSON(bytes.NewReader(raw))
	if err != nil {
		return invalid(path, "settings are not JSON values: %v", err)
	}
	if err := compiled.Validate(inst); err != nil {
		return invalid(path, "%v", err)
	}
	return nil
}
