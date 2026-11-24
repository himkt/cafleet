package core

func runtimeError(message string) *cafleetError { return appError(message) }

func ioError(cause string) *cafleetError { return runtimeError("member runtime: " + cause) }

func ensure(condition bool, message string) error {
	if condition {
		return nil
	}
	return runtimeError(message)
}

type terminalSize struct {
	rows uint16
	cols uint16
}

func (s terminalSize) validate() error {
	return ensure(s.rows > 0 && s.cols > 0, "terminal dimensions must be positive")
}
