package core

import (
	"embed"
)

func Main(args []string, _ embed.FS) int {
	return runArgs(args)
}
