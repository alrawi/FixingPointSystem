// SPDX-License-Identifier: GPL-3.0-only

package compiler

// Declarations holds the arrival and size declarations of a test
// configuration (test configuration v0.3, CON-19) in the form that the
// realism check reads as its targets. The declarations are targets only. They
// add no command to a plan, because the controller still schedules arrivals
// from the load objective and the phases and sizes still come from the
// scenarios and their params.
type Declarations struct {
	// Arrival is profile.arrival, the gaps between the starts of consecutive
	// sessions of the whole run.
	Arrival *Arrival `json:"arrival,omitempty"`
	// Size is profile.size, the default of every mix entry.
	Size *Size `json:"size,omitempty"`
	// Mix lists the entries that have a declaration of their own or that
	// inherit one from profile.size, in the order of the configuration.
	Mix []EntryDeclaration `json:"mix,omitempty"`
}

// EntryDeclaration is the declaration that holds for one mix entry.
type EntryDeclaration struct {
	// Scenario is the reference as the configuration writes it.
	Scenario string `json:"scenario"`
	// Arrival is the arrival of the entry itself. An entry without one has no
	// arrival process of its own, and does not take profile.arrival.
	Arrival *Arrival `json:"arrival,omitempty"`
	// Size is the size of the entry after its directions override the same
	// directions of profile.size.
	Size *Size `json:"size,omitempty"`
}

// Declarations returns the declarations of the configuration, or nil when it
// declares neither an arrival nor a size anywhere.
func (c Config) Declarations() *Declarations {
	d := &Declarations{Arrival: c.Profile.Arrival, Size: c.Profile.Size}
	for _, m := range c.Profile.Mix {
		e := EntryDeclaration{Scenario: m.Scenario, Arrival: m.Arrival, Size: mergeSize(c.Profile.Size, m.Size)}
		if e.Arrival != nil || e.Size != nil {
			d.Mix = append(d.Mix, e)
		}
	}
	if d.Arrival == nil && d.Size == nil && len(d.Mix) == 0 {
		return nil
	}
	return d
}

// mergeSize returns the size of a mix entry: each direction that the entry
// names overrides the same direction of the profile. It returns nil when no
// direction is declared.
func mergeSize(profile, entry *Size) *Size {
	var out Size
	pick := func(p, e *SizeDist) *SizeDist {
		if e != nil {
			return e
		}
		return p
	}
	if profile != nil {
		out = *profile
	}
	if entry != nil {
		out.Request = pick(out.Request, entry.Request)
		out.Response = pick(out.Response, entry.Response)
		out.Both = pick(out.Both, entry.Both)
	}
	if out.Request == nil && out.Response == nil && out.Both == nil {
		return nil
	}
	return &out
}
