package core

import (
	"crypto/sha256"
	"os"
	"path/filepath"
	"testing"
)

func sha256FileHex(path string) (string, error) {
	contents, err := os.ReadFile(path)
	if err != nil {
		return "", err
	}
	digest := sha256.Sum256(contents)
	return digestHex(digest[:]), nil
}

func TestDigest_HexWritesLowercaseTwoDigitBytes(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name  string
		bytes []byte
		want  string
	}{
		{name: "empty", bytes: nil, want: ""},
		{name: "leading zero nibbles", bytes: []byte{0x00, 0x0a, 0xff, 0xb0}, want: "000affb0"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			if got := digestHex(tc.bytes); got != tc.want {
				t.Fatalf("%s: digestHex = %q, want %q", tc.name, got, tc.want)
			}
		})
	}
}

func TestDigest_Sha256HexIsTheLowercaseDigest(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name  string
		input string
		want  string
	}{
		{name: "empty", input: "", want: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"},
		{name: "abc", input: "abc", want: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			if got := sha256Hex([]byte(tc.input)); got != tc.want {
				t.Fatalf("%s: sha256Hex = %q, want %q", tc.name, got, tc.want)
			}
		})
	}
	path := filepath.Join(t.TempDir(), "abc")
	if err := os.WriteFile(path, []byte("abc"), 0o600); err != nil {
		t.Fatal(err)
	}
	if got, err := sha256FileHex(path); err != nil || got != cases[1].want {
		t.Fatalf("sha256FileHex = %q %v", got, err)
	}
}

func TestDigest_CanonicalJsonSortsMembersByUtf8KeyBytesRecursively(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name  string
		input string
		want  string
	}{
		{
			name:  "nested objects and arrays",
			input: `{ "b" : "\u0041\/", "a" : [1, {"d": true, "c": null}], "A": -12 }`,
			want:  `{"A":-12,"a":[1,{"c":null,"d":true}],"b":"A/"}`,
		},
		{
			name:  "UTF-8 byte order rather than UTF-16 order",
			input: `{"\ud83d\ude00": 2, "\uff61": 1, "z": 0}`,
			want:  `{"z":0,"｡":1,"😀":2}`,
		},
		{
			name:  "SPEC string escaping",
			input: `{"s": "\u2028\u007f\u001f\b\t é"}`,
			want:  "{\"s\":\"\u2028\x7f\\u001f\\b\\t é\"}",
		},
		{
			name:  "integers",
			input: `[-9223372036854775808, 9223372036854775807, 18446744073709551615, 0]`,
			want:  `[-9223372036854775808,9223372036854775807,18446744073709551615,0]`,
		},
		{name: "scalar", input: ` "x" `, want: `"x"`},
		{name: "empty containers", input: `{"b": {}, "a": []}`, want: `{"a":[],"b":{}}`},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			got, err := canonicalJSON([]byte(tc.input))
			if err != nil || got != tc.want {
				t.Fatalf("%s: canonicalJSON = %q %v, want %q", tc.name, got, err, tc.want)
			}
		})
	}
}

func TestDigest_CanonicalJsonRejectsInvalidJson(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name  string
		input string
	}{
		{name: "truncated", input: `{"a":`},
		{name: "trailing data", input: `{} {}`},
		{name: "invalid UTF-8", input: "\"\xff\""},
		{name: "empty", input: ""},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			if got, err := canonicalJSON([]byte(tc.input)); err == nil {
				t.Fatalf("%s: canonicalJSON = %q", tc.name, got)
			}
		})
	}
}
