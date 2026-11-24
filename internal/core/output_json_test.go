package core

import (
	"strings"
	"testing"
)

func mustEncodeJSON(t *testing.T, value any) string {
	t.Helper()
	encoded, err := encodeJSON(value)
	if err != nil {
		t.Fatal(err)
	}
	return string(encoded)
}

func jsonPlacementFixture(concrete bool) placement {
	recorded := placement{
		backend:       "tmux",
		muxSession:    "main",
		muxWindowID:   none[string](),
		muxWindowName: none[string](),
		windowOrdinal: 1,
		columnSide:    columnSideLeft,
		muxPaneID:     none[string](),
		codingAgent:   "claude",
		createdAt:     "2026-11-13T00:00:00+00:00",
		muxServer:     "/tmp/tmux-1000/default",
	}
	if concrete {
		recorded.muxWindowID = some("@6")
		recorded.muxWindowName = some("cafleet-3-1")
		recorded.muxPaneID = some("%12")
	}
	return recorded
}

func TestOutputJson_PlacementJson_APlacementOrdersItsWindowAndSlotKeysBeforeThePaneAndNullsPendingIds(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name     string
		concrete bool
		want     string
	}{
		{
			name:     "concrete placement",
			concrete: true,
			want:     `{"backend":"tmux","mux_session":"main","mux_window_id":"@6","mux_window_name":"cafleet-3-1","window_ordinal":1,"column_side":"left","mux_pane_id":"%12","coding_agent":"claude","created_at":"2026-11-13T00:00:00+00:00"}`,
		},
		{
			name: "pending placement",
			want: `{"backend":"tmux","mux_session":"main","mux_window_id":null,"mux_window_name":null,"window_ordinal":1,"column_side":"left","mux_pane_id":null,"coding_agent":"claude","created_at":"2026-11-13T00:00:00+00:00"}`,
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			if got := mustEncodeJSON(t, outputJSONPlacement(jsonPlacementFixture(tc.concrete))); got != tc.want {
				t.Fatalf("%s: placement JSON\n%s\nwant\n%s", tc.name, got, tc.want)
			}
		})
	}
}

func TestOutputJson_PlacementJson_MemberActivityNamesTheLatestConsumptionLastConsumedBeforeIdle(t *testing.T) {
	t.Parallel()
	row := memberActivity{
		member: memberRecord{
			memberID:  4,
			fleetID:   1,
			name:      "worker",
			status:    memberStatusActive,
			kind:      memberKindMember,
			placement: jsonPlacementFixture(true),
		},
		lastSent:     none[string](),
		lastRecv:     some("2026-11-13T00:00:01+00:00"),
		lastConsumed: some("2026-11-13T00:00:02+00:00"),
		idle:         some(int64(3)),
	}
	placementJSON := mustEncodeJSON(t, outputJSONPlacement(jsonPlacementFixture(true)))
	want := `{"member_id":4,"name":"worker","kind":"member","placement":` + placementJSON +
		`,"last_sent":null,"last_recv":"2026-11-13T00:00:01+00:00","last_consumed":"2026-11-13T00:00:02+00:00","idle":3}`
	if got := mustEncodeJSON(t, outputJSONMemberActivity(row)); got != want {
		t.Fatalf("member activity JSON\n%s\nwant\n%s", got, want)
	}
}

func TestOutputJson_PlacementJson_ColumnSidesRenderAsTheirStoredValues(t *testing.T) {
	t.Parallel()
	cases := []struct {
		side columnSide
		wire string
	}{
		{columnSideLeft, `"column_side":"left"`},
		{columnSideRight, `"column_side":"right"`},
	}
	for _, tc := range cases {
		recorded := jsonPlacementFixture(true)
		recorded.columnSide = tc.side
		if got := mustEncodeJSON(t, outputJSONPlacement(recorded)); !strings.Contains(got, tc.wire) {
			t.Fatalf("%s: placement JSON %s", tc.wire, got)
		}
	}
}
