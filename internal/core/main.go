package core

import (
	"embed"
	"fmt"
	"os"
)

func Main(_ []string, _ embed.FS) int {
	fmt.Fprintf(os.Stderr, "Error: cafleet %s is not yet ported to Go\n", Version)
	return 1
}
