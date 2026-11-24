package core

import (
	"io"
	"os"
)

func readStdin() ([]byte, error) {
	return io.ReadAll(os.Stdin)
}
