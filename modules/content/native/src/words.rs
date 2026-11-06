// SPDX-License-Identifier: GPL-3.0-only
//! Small English word lists for page titles, link labels, and API records.
//! Every list was written for this module.

pub(crate) const NOUNS: &[&str] = &[
    "report", "ticket", "order", "invoice", "message", "project", "account", "device", "gateway",
    "sensor", "archive", "catalog", "schedule", "booking", "payment", "shipment", "review",
    "profile", "session", "channel", "record", "package", "service", "backup", "article",
    "gallery", "survey", "notice", "contract", "library",
];

pub(crate) const ADJECTIVES: &[&str] = &[
    "quiet", "bright", "steady", "narrow", "northern", "careful", "simple", "early", "common",
    "tall", "silver", "gentle", "rapid", "plain", "orderly", "central", "hidden", "tidy", "wide",
    "fresh", "calm", "sturdy", "modest", "clear",
];

pub(crate) const STATUSES: &[&str] = &[
    "active", "pending", "closed", "archived", "draft", "review", "paused",
];

pub(crate) const TAGS: &[&str] = &[
    "alpha", "beta", "gamma", "delta", "north", "south", "east", "west", "fast", "slow", "red",
    "green", "blue", "amber", "small", "large",
];
