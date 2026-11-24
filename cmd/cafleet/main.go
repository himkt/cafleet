package main

import (
	"os"

	"github.com/himkt/cafleet"
	"github.com/himkt/cafleet/internal/core"
)

func main() {
	os.Exit(core.Main(os.Args, cafleet.Assets))
}
