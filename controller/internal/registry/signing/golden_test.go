// SPDX-License-Identifier: GPL-3.0-only

package signing_test

// Golden values for the key with seed 0x07 repeated 32 times. Ed25519 is
// deterministic, so these bytes never change unless the format does.
const (
	goldenKeyID           = "ed25519:fe812c12f3ab4ce6ac5db69ac352f906"
	goldenDigestSignature = "b6077b349e70b3bd4901a0464136bfdab325845510be59c11f10baf577d9a9e10452b825683efb58dd7db1a75d3ce3dbbeb003193b374464a19a01692e8ffe0e"
	goldenCanonicalIndex  = `{"format":"fps-registry-index/1","modules":[{"digest":"sha256:0000000000000000000000000000000000000000000000000000000000000001","name":"dns","version":"0.1.0"},{"digest":"sha256:0000000000000000000000000000000000000000000000000000000000000001","name":"http1","version":"1.2.0"}],"serial":3}`
	goldenIndexSignature  = "0e2a7ec5532d0b3e9d8356be66ac5056ab8032ef179c8c4b43839e341bb40ef43ffc7a7cbccbdfaad44407f900f8ddc89fa2611b921aa6b2793bea615edf160c"
)
