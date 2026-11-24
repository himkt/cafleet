package core

// opt is an optional value; opt values compare equal when both are absent or both hold equal
// values.
type opt[T comparable] struct {
	value T
	set   bool
}

func some[T comparable](value T) opt[T] {
	return opt[T]{value: value, set: true}
}

func none[T comparable]() opt[T] {
	return opt[T]{}
}

func (o opt[T]) get() (T, bool) {
	return o.value, o.set
}
