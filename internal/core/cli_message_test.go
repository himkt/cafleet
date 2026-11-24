package core

import (
	"testing"
)

func TestCliMessage_MessageSubcommandsOutsideTheRegistryAreUnrecognized(t *testing.T) {
	t.Parallel()
	for _, args := range [][]string{
		{"message", "ack", "44"},
		{"message", "wait", "17"},
		{"message", "listen", "1"},
	} {
		assertParseError(t, args[1], args, args[1])
	}
}

func TestCliMessage_NextCommand_TheNextPollCommandAppendsTheDatabaseUrlQuotedOnlyWhenNeeded(t *testing.T) {
	t.Parallel()
	const everySafeClass = "sqlite:///Home/u_1/~x/a-b.c@d%e+f=g,h.db"
	cases := []struct {
		name   string
		member int64
		url    opt[string]
		want   string
	}{
		{name: "omitted database URL", member: 7, url: none[string](), want: "cafleet message poll 7 --json"},
		{name: "only safe characters", member: 7, url: some("sqlite:///tmp/fleet.db"), want: "cafleet message poll 7 --json --database-url sqlite:///tmp/fleet.db"},
		{name: "every safe character class", member: 42, url: some(everySafeClass), want: "cafleet message poll 42 --json --database-url " + everySafeClass},
		{name: "a space", member: 7, url: some("sqlite:///tmp/my fleet.db"), want: "cafleet message poll 7 --json --database-url 'sqlite:///tmp/my fleet.db'"},
		{name: "a single quote closed, escaped and reopened", member: 7, url: some("sqlite:///tmp/it's.db"), want: `cafleet message poll 7 --json --database-url 'sqlite:///tmp/it'\''s.db'`},
	}
	for _, tc := range cases {
		if got := pollCommand(tc.member, tc.url); got != tc.want {
			t.Fatalf("%s: pollCommand = %q, want %q", tc.name, got, tc.want)
		}
	}
}

func TestCliMessage_NextCommand_EveryCharacterOutsideTheSafeSetQuotesTheWholeUrl(t *testing.T) {
	t.Parallel()
	for _, unsafe := range []string{"$", ";", "*", `"`, `\`, "&", "|", "`", "#", "!", "\t", "é"} {
		url := "sqlite:///tmp/a" + unsafe + "b.db"
		want := "cafleet message poll 3 --json --database-url '" + url + "'"
		if got := pollCommand(3, some(url)); got != want {
			t.Fatalf("%q: pollCommand = %q, want %q", unsafe, got, want)
		}
	}
}
