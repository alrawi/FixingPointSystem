// SPDX-License-Identifier: GPL-3.0-only

package compiler

import (
	"fmt"
	"math"
	"sort"
)

// span is the slice of one pool that one agent owns.
type span struct {
	offset, count uint32
}

// splitPools splits every host pool across the workers of the agents that run
// its port, in global worker order. Each such worker gets the pool count
// divided by their number, and the first workers get one more host each until
// the remainder is used up. A pool with fewer hosts than workers leaves the
// last workers without hosts. Since the workers of an agent are contiguous in
// the global order, every agent owns a contiguous slice whose size follows its
// core count. onAgent says for each pool which agents run its port. It
// returns the host ranges of each worker and the slice of each pool for each
// agent. An agent that runs the port of a pool needs at least one host of it.
func splitPools(pools []HostPool, workers []worker, onAgent [][]bool) ([][]HostRange, [][]span, error) {
	hosts := make([][]HostRange, len(workers))
	slices := make([][]span, len(pools))
	for p, hp := range pools {
		owners := make([]int, 0, len(workers))
		for g, w := range workers {
			if onAgent[p][w.agent] {
				owners = append(owners, g)
			}
		}
		counts := evenSplit(uint64(hp.Count), len(owners), 0) //nolint:gosec // G115: count was checked to be positive
		slices[p] = make([]span, len(onAgent[p]))
		seen := make([]bool, len(onAgent[p]))
		var cursor uint64
		for i, g := range owners {
			w := workers[g]
			n := counts[i]
			if !seen[w.agent] {
				seen[w.agent] = true
				slices[p][w.agent].offset = uint32(cursor) //nolint:gosec // G115: cursor stays below the pool count, which fits uint32
			}
			if n == 0 {
				continue
			}
			hosts[g] = append(hosts[g], HostRange{Pool: hp.Name, Offset: uint32(cursor), Count: uint32(n)}) //nolint:gosec // G115: both stay below the pool count
			slices[p][w.agent].count += uint32(n)                                                           //nolint:gosec // G115: the sum stays below the pool count
			cursor += n
		}
		agents := 0
		for _, on := range onAgent[p] {
			if on {
				agents++
			}
		}
		for a, on := range onAgent[p] {
			if on && slices[p][a].count == 0 {
				return nil, nil, fmt.Errorf("%w: pool %s has %d hosts for %d workers on %d agents, so agent %d gets none",
					ErrPoolTooSmall, hp.Name, hp.Count, len(owners), agents, a)
			}
		}
	}
	return hosts, slices, nil
}

// evenSplit divides n into k parts that differ by at most one. The larger
// parts go to the k indices that follow start, wrapping around.
func evenSplit(n uint64, k, start int) []uint64 {
	parts := make([]uint64, k)
	base, rem := n/uint64(k), int(n%uint64(k)) //nolint:gosec // G115: k is positive and rem is below k
	for i := range parts {
		parts[i] = base
		if (i-start+k)%k < rem {
			parts[i]++
		}
	}
	return parts
}

// splitRates computes the steady state target of every mix entry on every
// worker and the objective value of every agent. eligible lists for each mix
// entry the global indexes of the workers that own hosts of its client pool.
// The entry divides its share of the objective equally over these workers, and
// the other workers get no rate for it, so the total rate stays as configured.
// When every worker is eligible for every entry, every worker carries an equal
// share and the share of an agent follows its core count.
func splitRates(obj Objective, mix []MixEntry, estimates []float64, workers []worker, cores []uint32, eligible [][]int) ([][]WorkerRate, []float64, error) {
	k := len(workers)
	rates := make([][]WorkerRate, k)
	value := make([]float64, len(cores))
	switch obj.Type {
	case "sessions_per_second", "throughput_bps":
		total := obj.Value
		if obj.Type == "throughput_bps" {
			// Sessions per second S move S times the weighted mean bytes per
			// session, times eight bits, which must equal the objective.
			var bytesPerSession float64
			for i, e := range mix {
				if !(estimates[i] > 0) || math.IsInf(estimates[i], 0) {
					return nil, nil, fmt.Errorf("%w: scenario %s", ErrNoByteEstimate, e.Scenario)
				}
				bytesPerSession += e.Weight * estimates[i]
			}
			total = obj.Value / (8 * bytesPerSession)
		}
		full := true
		for i, e := range mix {
			if len(eligible[i]) != k {
				full = false
			}
			r := total * e.Weight / float64(len(eligible[i]))
			for _, g := range eligible[i] {
				rate := r
				rates[g] = append(rates[g], WorkerRate{MixEntry: e.ID, SessionsPerSecond: &rate})
			}
		}
		if full {
			for a, c := range cores {
				value[a] = obj.Value * float64(c) / float64(k)
			}
			break
		}
		// An entry carries its share of the objective, which is its session
		// weight, or its byte weight for a throughput objective. The share
		// splits equally over the eligible workers.
		share := make([]float64, len(mix))
		var sum float64
		for i, e := range mix {
			share[i] = e.Weight
			if obj.Type == "throughput_bps" {
				share[i] *= estimates[i]
			}
			sum += share[i]
		}
		for i := range mix {
			for _, g := range eligible[i] {
				value[workers[g].agent] += obj.Value * share[i] / sum / float64(len(eligible[i]))
			}
		}
	default:
		// Concurrency objectives keep whole sessions. A simulated user holds
		// one session at a time.
		n := math.Round(obj.Value)
		if n < 1 || n > math.MaxUint32 {
			return nil, nil, invalid("load.objective.value", "%v does not round to a session count from 1 to %d", obj.Value, uint32(math.MaxUint32))
		}
		weights := make([]float64, len(mix))
		for i, e := range mix {
			weights[i] = e.Weight
		}
		cursor := 0
		for i, t := range largestRemainder(uint64(n), weights) {
			el := eligible[i]
			parts := evenSplit(t, len(el), cursor%len(el))
			cursor = (cursor + int(t%uint64(len(el)))) % k //nolint:gosec // G115: the remainder is below the worker count
			for j, g := range el {
				s := uint32(parts[j]) //nolint:gosec // G115: parts never exceed n, which fits uint32
				rates[g] = append(rates[g], WorkerRate{MixEntry: mix[i].ID, ConcurrentSessions: &s})
				value[workers[g].agent] += float64(s)
			}
		}
	}
	return rates, value, nil
}

// largestRemainder apportions n whole units by weight. Each part gets the
// floor of its quota, and the units left over go to the largest fractional
// remainders, with ties going to the earlier index.
func largestRemainder(n uint64, weights []float64) []uint64 {
	var sum float64
	for _, w := range weights {
		sum += w
	}
	parts := make([]uint64, len(weights))
	frac := make([]float64, len(weights))
	var given uint64
	for i, w := range weights {
		q := float64(n) * w / sum
		parts[i] = uint64(math.Floor(q))
		frac[i] = q - math.Floor(q)
		given += parts[i]
	}
	order := make([]int, len(weights))
	for i := range order {
		order[i] = i
	}
	sort.SliceStable(order, func(a, b int) bool { return frac[order[a]] > frac[order[b]] })
	// Rounding in the quotas can overshoot by a unit, which the smallest
	// remainders give back.
	for i := len(order) - 1; given > n; i = (i - 1 + len(order)) % len(order) {
		if parts[order[i]] > 0 {
			parts[order[i]]--
			given--
		}
	}
	for i := 0; given < n; i = (i + 1) % len(order) {
		parts[order[i]]++
		given++
	}
	return parts
}
