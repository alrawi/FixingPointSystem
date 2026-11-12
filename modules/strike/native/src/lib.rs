//! Native support of the strike module.
//!
//! The scenarios of the module are pure contract programs: the template
//! engine evaluates them and the kernel backend runs the sockets, so the
//! module has no native template functions. The crate holds the fixture
//! peer example, which `../fixtures/capture.sh` runs in the QA-01 harness
//! to record the module fixture.
