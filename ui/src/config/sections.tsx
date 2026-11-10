// SPDX-License-Identifier: GPL-3.0-only
//
// The form sections of the configuration editor, one per top level section of
// the test configuration schema: general fields, network, profile, load,
// timeline, and criteria. Fields that the forms do not cover, such as
// `metadata.labels`, `modules`, `params`, and `where`, stay in the YAML text
// untouched, and the forms point to the YAML view for them.

import type { ReactNode } from "react";
import { Field, ItemList, SelectField, YamlOnly } from "./fields";
import { useConfigModel } from "./model";

function Section({ id, title, children }: { id: string; title: string; children: ReactNode }) {
  return (
    <section aria-labelledby={id} className="editor-section">
      <h2 id={id}>{title}</h2>
      {children}
    </section>
  );
}

/** Name, description, and topology. */
export function GeneralSection() {
  return (
    <Section id="sec-general" title="General">
      <div className="item-fields">
        <Field path={["metadata", "name"]} label="Name" />
        <Field path={["metadata", "description"]} label="Description" optional />
        <SelectField
          path={["topology"]}
          label="Topology"
          options={["app_sim", "client_sim", "server_sim"]}
          hint="app_sim emulates clients and servers, client_sim only clients, and server_sim only servers."
        />
      </div>
      <YamlOnly path={["metadata", "labels"]} label="Labels" />
      <YamlOnly path={["modules"]} label="Module settings" />
    </Section>
  );
}

/** Ports, host pools, and external servers. */
export function NetworkSection() {
  const model = useConfigModel();
  const portNames = (() => {
    const ports = model.get(["network", "ports"]);
    return Array.isArray(ports)
      ? ports.flatMap((port: unknown) =>
          typeof port === "object" &&
          port !== null &&
          "name" in port &&
          typeof port.name === "string"
            ? [port.name]
            : [],
        )
      : [];
  })();
  return (
    <Section id="sec-network" title="Network">
      <h3>Ports</h3>
      <ItemList
        path={["network", "ports"]}
        noun="port"
        create={(index) => ({ name: `port${String(index)}`, location: "local/lo" })}
      >
        {(_, path) => (
          <>
            <Field path={[...path, "name"]} label="Name" />
            <Field
              path={[...path, "location"]}
              label="Location"
              hint="agent/interface pins the port to one agent. A bare interface spreads it across agents."
            />
            <Field path={[...path, "mtu"]} label="MTU" kind="integer" optional />
          </>
        )}
      </ItemList>
      <h3>Host pools</h3>
      <ItemList
        path={["network", "host_pools"]}
        noun="host pool"
        create={(index) => ({
          name: `pool${String(index)}`,
          port: portNames[0] ?? "",
          role: "client",
          ipv4: "10.0.0.0/24",
          count: 1,
        })}
      >
        {(_, path) => (
          <>
            <Field path={[...path, "name"]} label="Name" />
            <Field path={[...path, "port"]} label="Port" suggestions={portNames} />
            <SelectField path={[...path, "role"]} label="Role" options={["client", "server"]} />
            <Field
              path={[...path, "ipv4"]}
              label="IPv4 range"
              optional
              hint="For example 10.0.0.0/24."
            />
            <Field path={[...path, "ipv6"]} label="IPv6 range" optional />
            <Field path={[...path, "count"]} label="Host count" kind="integer" />
            <Field path={[...path, "gateway"]} label="Gateway" optional />
            <Field path={[...path, "vlan"]} label="VLAN" kind="integer" optional />
            <Field path={[...path, "mac_base"]} label="MAC base" optional />
            <Field path={[...path, "os_profile"]} label="OS profile" optional />
          </>
        )}
      </ItemList>
      <h3>External servers</h3>
      <ItemList
        path={["network", "external_servers"]}
        noun="external server"
        create={(index) => ({ name: `server${String(index)}`, address: "" })}
      >
        {(_, path) => (
          <>
            <Field path={[...path, "name"]} label="Name" />
            <Field path={[...path, "address"]} label="Address" />
            <Field path={[...path, "port"]} label="Port" kind="integer" optional />
          </>
        )}
      </ItemList>
    </Section>
  );
}

/** The profile basis and the mix of scenarios. */
export function ProfileSection() {
  const { scenarios } = useConfigModel();
  return (
    <Section id="sec-profile" title="Profile">
      <SelectField
        path={["profile", "basis"]}
        label="Mix basis"
        options={["sessions", "bytes"]}
        hint="Weights share the sessions or the bytes of the run."
      />
      <h3>Mix</h3>
      <ItemList
        path={["profile", "mix"]}
        noun="scenario"
        create={() => ({ scenario: "", weight: 1 })}
      >
        {(_, path) => (
          <>
            <Field
              path={[...path, "scenario"]}
              label="Scenario"
              suggestions={scenarios}
              hint="module/scenario, or a scenario name that one installed module provides."
            />
            <Field path={[...path, "weight"]} label="Weight" kind="number" />
            <Field path={[...path, "client_pool"]} label="Client pool" optional />
            <Field path={[...path, "server_pool"]} label="Server pool" optional />
            <YamlOnly path={[...path, "params"]} label="Parameters" />
          </>
        )}
      </ItemList>
    </Section>
  );
}

/** The load objective and its phases. */
export function LoadSection() {
  return (
    <Section id="sec-load" title="Load">
      <div className="item-fields">
        <SelectField
          path={["load", "objective", "type"]}
          label="Objective"
          options={[
            "concurrent_sessions",
            "sessions_per_second",
            "throughput_bps",
            "simulated_users",
          ]}
        />
        <Field path={["load", "objective", "value"]} label="Objective value" kind="number" />
      </div>
      <h3>Phases</h3>
      <ItemList
        path={["load", "phases"]}
        noun="phase"
        create={(index) => ({ name: `phase${String(index)}`, duration_s: 60, shape: "hold" })}
      >
        {(_, path) => (
          <>
            <Field path={[...path, "name"]} label="Name" />
            <Field path={[...path, "duration_s"]} label="Duration in seconds" kind="number" />
            <SelectField
              path={[...path, "shape"]}
              label="Shape"
              options={["linear", "step", "hold"]}
              optional
              hint="Holds the level when not set."
            />
          </>
        )}
      </ItemList>
    </Section>
  );
}

/** The seed and the warmup. */
export function TimelineSection() {
  return (
    <Section id="sec-timeline" title="Timeline">
      <div className="item-fields">
        <Field
          path={["timeline", "seed"]}
          label="Seed"
          kind="integer"
          optional
          hint="Leave it empty and the controller chooses a random seed and records it in the run."
        />
        <Field path={["timeline", "warmup_s"]} label="Warmup in seconds" kind="number" optional />
      </div>
    </Section>
  );
}

/** The pass or fail criteria. */
export function CriteriaSection() {
  return (
    <Section id="sec-criteria" title="Criteria">
      <ItemList
        path={["criteria"]}
        noun="criterion"
        create={() => ({ metric: "sessions.failed_ratio", op: "<", value: 0.01 })}
      >
        {(_, path) => (
          <>
            <Field
              path={[...path, "metric"]}
              label="Metric"
              hint="A metric of the catalog, for example sessions.failed_ratio."
            />
            <SelectField
              path={[...path, "op"]}
              label="Operator"
              options={["<", "<=", ">", ">=", "==", "!="]}
            />
            <Field path={[...path, "value"]} label="Value" kind="number" />
            <YamlOnly path={[...path, "where"]} label="Label filter" />
          </>
        )}
      </ItemList>
    </Section>
  );
}
