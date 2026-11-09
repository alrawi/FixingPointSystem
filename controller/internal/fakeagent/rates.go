// SPDX-License-Identifier: GPL-3.0-only

package fakeagent

import (
	"fmt"
	"math"
	"strings"
	"sync"
	"time"

	"github.com/alrawi/FixingPointSystem/controller/internal/agents/agentpb"
)

// Traffic turns on the traffic simulation of a fake agent. While the agent
// runs a plan, it completes the sessions that the session rates of the plan
// call for, each carrying BytesPerSession bytes, and sends the client side
// counters sessions.attempted, sessions.succeeded, bytes.tx, and bytes.rx of
// each scenario in a snapshot per tick. UpdateRates changes the rates that the
// simulation runs, so the bytes of the agent follow the commands of the
// controller, as they do with a real engine.
type Traffic struct {
	// BytesPerSession is the number of payload bytes of one completed session,
	// counted once, half as bytes.tx and half as bytes.rx. It must be positive.
	BytesPerSession float64
	// BytesByScenario overrides BytesPerSession for the scenarios that it
	// names, by scenario name such as web_get, so one agent can run scenarios
	// whose sessions carry very different bytes. The values must be positive.
	BytesByScenario map[string]float64
	// Speed is the number of simulated seconds that pass per real second.
	// Zero means 1.
	Speed float64
	// Tick is the real time between two snapshots. Zero means 5 ms.
	Tick time.Duration
}

// scenarioKey names the scenario of a mix entry by its application and name.
type scenarioKey struct{ application, name string }

// simTotals are the cumulative client side counters of one scenario.
type simTotals struct{ sessions, bytes uint64 }

// trafficSim is the rate state of an agent and its traffic simulation. The
// rate state follows the plan and UpdateRates even without a Traffic option.
type trafficSim struct {
	mu         sync.Mutex
	rates      map[string]float64 // sessions per second of the whole agent, by mix entry
	concurrent map[string]uint32  // concurrent sessions of the whole agent, by mix entry
	scenario   map[string]scenarioKey
	order      []string // mix entries in plan order
	acc        map[string]float64
	totals     map[scenarioKey]*simTotals
	bytes      uint64
	simTime    float64
	stop       chan struct{}
	done       chan struct{}
}

// load sets the rates of a freshly prepared plan: the sum of the rates of its
// workers, by mix entry.
func (s *trafficSim) load(plan *agentpb.Plan) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.rates, s.concurrent = map[string]float64{}, map[string]uint32{}
	s.scenario, s.acc, s.totals, s.order = map[string]scenarioKey{}, map[string]float64{}, map[scenarioKey]*simTotals{}, nil
	s.bytes, s.simTime = 0, 0
	for _, m := range plan.GetMix() {
		app, name, _ := strings.Cut(m.GetScenario(), "/")
		if name == "" {
			app, name = "", app
		}
		s.scenario[m.GetId()] = scenarioKey{app, name}
		s.rates[m.GetId()] = 0
		s.order = append(s.order, m.GetId())
	}
	for _, w := range plan.GetWorkers() {
		for _, r := range w.GetRates() {
			if _, ok := s.rates[r.GetMixEntry()]; !ok {
				continue
			}
			s.rates[r.GetMixEntry()] += r.GetSessionsPerSecond()
			s.concurrent[r.GetMixEntry()] += r.GetConcurrentSessions()
		}
	}
}

// update applies the command completely or not at all.
func (s *trafficSim) update(cmd *agentpb.UpdateRatesCommand) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	for _, r := range cmd.GetRates() {
		if _, ok := s.rates[r.GetMixEntry()]; !ok {
			return fmt.Errorf("mix entry %q is not a mix entry of the plan", r.GetMixEntry())
		}
		switch r.GetTarget().(type) {
		case *agentpb.RateUpdate_SessionsPerSecond:
			if v := r.GetSessionsPerSecond(); v < 0 || math.IsNaN(v) || math.IsInf(v, 0) {
				return fmt.Errorf("mix entry %q: sessions per second %v is not a rate", r.GetMixEntry(), v)
			}
		case *agentpb.RateUpdate_ConcurrentSessions:
		default:
			return fmt.Errorf("mix entry %q has no target", r.GetMixEntry())
		}
	}
	for _, r := range cmd.GetRates() {
		switch r.GetTarget().(type) {
		case *agentpb.RateUpdate_SessionsPerSecond:
			s.rates[r.GetMixEntry()] = r.GetSessionsPerSecond()
		case *agentpb.RateUpdate_ConcurrentSessions:
			s.concurrent[r.GetMixEntry()] = r.GetConcurrentSessions()
		}
	}
	return nil
}

// Rates returns the session rate of the whole agent that it runs, in sessions
// per second by mix entry. It starts as the sum of the rates of the workers of
// the prepared plan, and UpdateRates replaces the entries that it names.
func (a *Agent) Rates() map[string]float64 {
	a.sim.mu.Lock()
	defer a.sim.mu.Unlock()
	out := make(map[string]float64, len(a.sim.rates))
	for k, v := range a.sim.rates {
		out[k] = v
	}
	return out
}

// TrafficTotals returns the bytes that the simulation completed so far and the
// simulated seconds that it ran. Both are zero without Options.Traffic.
func (a *Agent) TrafficTotals() (bytes uint64, simSeconds float64) {
	a.sim.mu.Lock()
	defer a.sim.mu.Unlock()
	return a.sim.bytes, a.sim.simTime
}

// ScenarioBytes returns the payload bytes that the simulation completed so far,
// by scenario name. It is empty without Options.Traffic.
func (a *Agent) ScenarioBytes() map[string]uint64 {
	a.sim.mu.Lock()
	defer a.sim.mu.Unlock()
	out := make(map[string]uint64, len(a.sim.totals))
	for key, t := range a.sim.totals {
		out[key.name] += t.bytes
	}
	return out
}

// startTraffic starts the traffic simulation of the running plan. It does
// nothing without Options.Traffic.
func (a *Agent) startTraffic() {
	t := a.opts.Traffic
	if t == nil || t.BytesPerSession <= 0 {
		return
	}
	a.StopTraffic()
	speed, tick := t.Speed, t.Tick
	if speed <= 0 {
		speed = 1
	}
	if tick <= 0 {
		tick = 5 * time.Millisecond
	}
	stop, done, session := make(chan struct{}), make(chan struct{}), a.Done()
	a.sim.mu.Lock()
	a.sim.stop, a.sim.done = stop, done
	a.sim.mu.Unlock()
	go func() {
		defer close(done)
		tk := time.NewTicker(tick)
		defer tk.Stop()
		for {
			select {
			case <-stop:
				return
			case <-session:
				return
			case <-tk.C:
			}
			if st, _ := a.State(); st != agentpb.AgentState_AGENT_STATE_RUNNING {
				return
			}
			counters := a.sim.step(tick.Seconds()*speed, t.BytesPerSession, t.BytesByScenario)
			if _, err := a.Snapshot(counters...); err != nil {
				return
			}
		}
	}()
}

// StopTraffic ends the traffic simulation and waits for it. It may be called
// more than once, and without a simulation it does nothing. Finish and a stop
// command call it before they send the final snapshot.
func (a *Agent) StopTraffic() {
	a.sim.mu.Lock()
	stop, done := a.sim.stop, a.sim.done
	a.sim.stop, a.sim.done = nil, nil
	a.sim.mu.Unlock()
	if stop == nil {
		return
	}
	close(stop)
	<-done
}

// step advances the simulation by dt simulated seconds and returns the
// cumulative client side counters.
func (s *trafficSim) step(dt, bytesPerSession float64, byScenario map[string]float64) []*agentpb.CounterValue {
	s.mu.Lock()
	defer s.mu.Unlock()
	for _, id := range s.order {
		s.acc[id] += s.rates[id] * dt
		whole := math.Floor(s.acc[id])
		s.acc[id] -= whole
		key := s.scenario[id]
		per := bytesPerSession
		if b, ok := byScenario[key.name]; ok && b > 0 {
			per = b
		}
		t := s.totals[key]
		if t == nil {
			t = &simTotals{}
			s.totals[key] = t
		}
		n := uint64(whole)
		t.sessions += n
		t.bytes += uint64(float64(n) * per)
		s.bytes += uint64(float64(n) * per)
	}
	s.simTime += dt
	var out []*agentpb.CounterValue
	for key, t := range s.totals {
		labels := map[string]string{"application": key.application, "scenario": key.name, "side": "client"}
		if key.application == "" {
			delete(labels, "application")
		}
		count := func(metric string, n uint64) *agentpb.CounterValue {
			return &agentpb.CounterValue{Metric: metric, Labels: labels, Value: &agentpb.CounterValue_Count{Count: n}}
		}
		out = append(out,
			count("sessions.attempted", t.sessions),
			count("sessions.succeeded", t.sessions),
			count("bytes.tx", t.bytes/2),
			count("bytes.rx", t.bytes-t.bytes/2),
		)
	}
	return out
}
