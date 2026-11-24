package core

import (
	"bytes"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json/jsontext"
	"fmt"
	"io"
	"slices"
	"strings"
)

func digestHex(data []byte) string {
	return hex.EncodeToString(data)
}

func sha256Hex(data []byte) string {
	sum := sha256.Sum256(data)
	return digestHex(sum[:])
}

// canonicalJSON renders one JSON value with every object's members sorted by the UTF-8 bytes of
// their keys, recursively, in the SPEC §6.4 JSON encoding.
func canonicalJSON(value []byte) (string, error) {
	decoder := jsontext.NewDecoder(bytes.NewReader(value))
	out, err := appendCanonicalJSON(nil, decoder)
	if err != nil {
		return "", err
	}
	if _, err := decoder.ReadToken(); err != io.EOF {
		return "", appErrorf("canonical JSON: trailing data after one JSON value: %v", err)
	}
	return string(out), nil
}

func appendCanonicalJSON(out []byte, decoder *jsontext.Decoder) ([]byte, error) {
	token, err := decoder.ReadToken()
	if err != nil {
		return nil, appErrorf("canonical JSON: %v", err)
	}
	switch token.Kind() {
	case '{':
		type member struct {
			key   string
			value []byte
		}
		var members []member
		for decoder.PeekKind() != '}' {
			name, err := decoder.ReadToken()
			if err != nil {
				return nil, appErrorf("canonical JSON: %v", err)
			}
			key := name.String()
			value, err := appendCanonicalJSON(nil, decoder)
			if err != nil {
				return nil, err
			}
			members = append(members, member{key: key, value: value})
		}
		if _, err := decoder.ReadToken(); err != nil {
			return nil, appErrorf("canonical JSON: %v", err)
		}
		slices.SortFunc(members, func(a, b member) int { return strings.Compare(a.key, b.key) })
		out = append(out, '{')
		for index, m := range members {
			if index > 0 {
				out = append(out, ',')
			}
			key, err := encodeJSON(m.key)
			if err != nil {
				return nil, err
			}
			out = append(append(out, key...), ':')
			out = append(out, m.value...)
		}
		return append(out, '}'), nil
	case '[':
		out = append(out, '[')
		for first := true; decoder.PeekKind() != ']'; first = false {
			if !first {
				out = append(out, ',')
			}
			out, err = appendCanonicalJSON(out, decoder)
			if err != nil {
				return nil, err
			}
		}
		if _, err := decoder.ReadToken(); err != nil {
			return nil, appErrorf("canonical JSON: %v", err)
		}
		return append(out, ']'), nil
	case '"':
		encoded, err := encodeJSON(token.String())
		if err != nil {
			return nil, err
		}
		return append(out, encoded...), nil
	case '0':
		return appendJSONInteger(out, token.String())
	case 't', 'f', 'n':
		return append(out, token.String()...), nil
	case jsontext.KindInvalid, jsontext.KindEndObject, jsontext.KindEndArray:
		return nil, appError(fmt.Sprintf("canonical JSON: a value cannot start with %v", token.Kind()))
	default:
		panic("invalid jsontext.Kind")
	}
}
