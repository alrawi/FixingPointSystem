module github.com/alrawi/FixingPointSystem/controller

go 1.24

require (
	github.com/alrawi/FixingPointSystem/tests/golden/go v0.0.0
	go.yaml.in/yaml/v3 v3.0.4
)

replace github.com/alrawi/FixingPointSystem/tests/golden/go => ../tests/golden/go
