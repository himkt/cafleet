package core

import (
	"strconv"
	"testing"
	"time"
)

func TestCliMessage_RunnerPull_MessageSendStoresTheMessageAndWritesNoByteToTheRecipientsChild(t *testing.T) {
	t.Parallel()
	withInputService(t, func(f *serviceFixture, _ time.Time) {
		run := f.fixture.run
		code := runArgs([]string{
			"cafleet", "--database-url", f.fixture.url, "message", "send",
			"--from-member-id", strconv.FormatInt(run.directorMemberID, 10),
			"--to-member-id", strconv.FormatInt(run.memberID, 10), "stored for the runner",
		})
		if code != 0 {
			t.Fatalf("message send exits %d", code)
		}
		if records := inputServiceRecords(t, f); len(records) != 0 {
			t.Fatalf("the recipient's child received %q", records)
		}
		status, err := queryRow(f.fixture.conn,
			"SELECT status_state,notified_at FROM messages WHERE to_member_id=?1 AND text='stored for the runner'",
			[]any{run.memberID}, func(row sqlRow) ([2]opt[string], error) {
				state, err := row.text(0)
				if err != nil {
					return [2]opt[string]{}, err
				}
				notified, err := row.optText(1)
				return [2]opt[string]{some(state), notified}, err
			})
		if err != nil {
			t.Fatal(err)
		}
		if status != [2]opt[string]{some("input_required"), none[string]()} {
			t.Fatalf("stored message status %v, want input_required and never notified", status)
		}
	})
}

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
