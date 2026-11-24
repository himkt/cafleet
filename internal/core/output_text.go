package core

import (
	"fmt"
	"strconv"
	"strings"
	"unicode"
	"unicode/utf8"
)

// quoted renders s as Rust's str Debug does (F9, SPEC §6.4 Quoted text): wrapped in `"`, with
// `"`, `\`, tab, newline, carriage return and NUL escaped by name and every scalar that is not
// printable or extends a grapheme escaped as \u{H}.
func quoted(s string) string {
	if !utf8.ValidString(s) {
		panic("quoted text is valid UTF-8")
	}
	var out strings.Builder
	out.WriteByte('"')
	for _, r := range s {
		switch r {
		case '"':
			out.WriteString(`\"`)
		case '\\':
			out.WriteString(`\\`)
		case '\t':
			out.WriteString(`\t`)
		case '\n':
			out.WriteString(`\n`)
		case '\r':
			out.WriteString(`\r`)
		case 0:
			out.WriteString(`\0`)
		default:
			if quotedEscapes(r) {
				out.WriteString(`\u{` + strconv.FormatInt(int64(r), 16) + `}`)
			} else {
				out.WriteRune(r)
			}
		}
	}
	out.WriteByte('"')
	return out.String()
}

// quotedEscapes holds for a scalar of general category Cc, Cf, Cs, Co, Cn, Zl, Zp or Zs other
// than U+0020, and for one with the Grapheme_Extend property (Mn, Me and
// Other_Grapheme_Extend).
func quotedEscapes(r rune) bool {
	switch {
	case r == ' ':
		return false
	case unicode.In(r, unicode.Cc, unicode.Cf, unicode.Cs, unicode.Co, unicode.Zl, unicode.Zp, unicode.Zs):
		return true
	case unicode.In(r, unicode.Mn, unicode.Me, unicode.Other_Grapheme_Extend):
		return true
	default:
		assigned := unicode.In(r, unicode.L, unicode.M, unicode.N, unicode.P, unicode.S, unicode.Z,
			unicode.Cc, unicode.Cf, unicode.Co, unicode.Cs)
		return !assigned
	}
}

const textBodyLimit = 200

func formatMessage(message messageRecord) string {
	header := fmt.Sprintf("[%d | from:%d | %s]", message.messageID, message.fromMemberID, message.statusTimestamp)
	return header + "\n" + truncateText(message.text, textBodyLimit)
}

func formatMessages(messages []messageRecord) string {
	if len(messages) == 0 {
		return "No messages found."
	}
	rendered := make([]string, 0, len(messages))
	for _, message := range messages {
		rendered = append(rendered, formatMessage(message))
	}
	return strings.Join(rendered, "\n\n")
}

func formatMessageSent(message messageRecord) string {
	return "Message sent.\n" + formatMessage(message)
}

func formatFleetList(fleets []fleetSummary) string {
	if len(fleets) == 0 {
		return "No fleets found."
	}
	lines := []string{"FLEET_ID DIRECTOR NAME MEMBERS CREATED_AT"}
	for _, fleet := range fleets {
		lines = append(lines, fmt.Sprintf("%d %d %s %d %s", fleet.fleetID, fleet.directorMemberID, fleet.name, fleet.memberCount, fleet.createdAt))
	}
	return strings.Join(lines, "\n")
}

func formatFleetShow(fleet fleetRecord) string {
	text := fmt.Sprintf("fleet_id: %d\nname: %s\ncreated_at: %s", fleet.fleetID, fleet.name, fleet.createdAt)
	if deleted, set := fleet.deletedAt.get(); set {
		text += "\ndeleted_at: " + deleted
	}
	return text
}

func formatMemberDelete(memberID int64, paneStatus string) string {
	return fmt.Sprintf("Member deleted.\n  member_id: %d\n  pane_id:   %s", memberID, paneStatus)
}

const pending = "(pending)"

func withDelivery(text string, inputID opt[int64], delivery deliveryProjection) string {
	method := "-"
	if value, set := delivery.Method.get(); set {
		method = value.asStr()
	}
	reason, set := delivery.Reason.get()
	if !set {
		reason = "-"
	}
	text += fmt.Sprintf("\nDelivery: status=%s method=%s reason=%s", delivery.Status.asStr(), method, reason)
	if id, set := inputID.get(); set {
		text += fmt.Sprintf("\nInput: %d", id)
		if delivery.Status == deliveryStatusQueued {
			text += fmt.Sprintf("\nThe member's runner will retry it after the blocking work ends; run cafleet input show %d --json for its result.", id)
		}
	}
	return text
}

func formatMemberPrompt(memberID int64, text, name, pane string, inputID opt[int64], delivery deliveryProjection) string {
	first := fmt.Sprintf("Sent prompt %s to member %s (%s).", quoted(text), name, pane)
	if delivery.Status == deliveryStatusQueued {
		first = fmt.Sprintf("Queued prompt for member %d (%s).", memberID, pane)
	}
	return withDelivery(first, inputID, delivery)
}

func formatMemberExec(memberID int64, pane string, inputID opt[int64], delivery deliveryProjection) string {
	first := fmt.Sprintf("Dispatched native command to member %d (%s). Inspect the pane for its result.", memberID, pane)
	if delivery.Status == deliveryStatusQueued {
		first = fmt.Sprintf("Queued native command for member %d (%s).", memberID, pane)
	}
	return withDelivery(first, inputID, delivery)
}

// windowLabel is the placement's window name, or (pending) while creation has not published it.
func windowLabel(p placement) string {
	if name, set := p.muxWindowName.get(); set {
		return name
	}
	return pending
}

func formatMemberDetail(member memberRecord) string {
	text := fmt.Sprintf("%d %s %s window=%s", member.memberID, member.name, member.status.asStr(), windowLabel(member.placement))
	if runtime, set := member.runtime.get(); set {
		text += "\n" + formatRuntimeSnapshot(*runtime)
	}
	return text
}

func formatFleetCreate(fleet createdFleet) string {
	return fmt.Sprintf("%d director=%d", fleet.fleetID, fleet.director.memberID)
}

func formatFleetDelete(deleted deletedFleet) string {
	lines := []string{fmt.Sprintf("Deleted fleet %d. Deregistered %d members.", deleted.fleetID, deleted.deregisteredCount)}
	for _, member := range deleted.members {
		lines = append(lines, fmt.Sprintf("  member %d: %s", member.memberID, member.paneStatus))
	}
	return strings.Join(lines, "\n")
}

func formatMember(member registeredMember, p placement) string {
	pane, set := p.muxPaneID.get()
	if !set {
		pane = pending
	}
	return fmt.Sprintf("%d %s backend=%s pane=%s window=%s", member.memberID, member.name, p.codingAgent, pane, windowLabel(p))
}

func formatIdle(idle opt[int64]) string {
	seconds, set := idle.get()
	switch {
	case !set:
		return "-"
	case seconds < 0:
		panic("member idle is nonnegative")
	case seconds < 60:
		return strconv.FormatInt(seconds, 10) + "s"
	case seconds < 3600:
		return strconv.FormatInt(seconds/60, 10) + "m"
	default:
		return strconv.FormatInt(seconds/3600, 10) + "h"
	}
}

func formatMemberList(members []memberActivity) string {
	if len(members) == 0 {
		return "0 members."
	}
	plural := ""
	if len(members) > 1 {
		plural = "s"
	}
	lines := []string{
		fmt.Sprintf("%d member%s:", len(members), plural),
		"  member_id  name           kind      backend   window         pane_id  idle",
	}
	for _, row := range members {
		member := row.member
		pane, set := member.placement.muxPaneID.get()
		if !set {
			pane = pending
		}
		lines = append(lines, fmt.Sprintf("  %-9d  %-13s  %-8s  %-8s  %-13s  %-7s  %s",
			member.memberID, member.name, member.kind.asStr(), member.placement.codingAgent,
			windowLabel(member.placement), pane, formatIdle(row.idle)))
	}
	return strings.Join(lines, "\n")
}

func formatRuntimeSnapshot(snapshot runtimeSnapshot) string {
	run := snapshot.run
	return fmt.Sprintf("Runtime: run=%d mode=%s status=%s coverage=%s", run.runID, run.resolvedMode, run.status.asStr(), run.coverage.asStr())
}

func formatRuntimeEvent(view runtimeEventView) string {
	event := view.event
	message := "-"
	if id, set := event.messageID.get(); set {
		message = strconv.FormatInt(id, 10)
	}
	text := fmt.Sprintf("Event: %d run=%d seq=%d kind=%s notification=%s message=%s\n%s",
		event.eventID, event.runID, event.eventSeq, event.kind.asStr(), event.notificationState.asStr(), message,
		formatRuntimeSnapshot(view.run))
	if interaction, set := view.interaction.get(); set {
		closedAt, closed := interaction.closedAt.get()
		if !closed {
			closedAt = "-"
		}
		closeReason := "-"
		if reason, set := interaction.closeReason.get(); set {
			closeReason = reason.asStr()
		}
		text += fmt.Sprintf("\nInteraction: %d classification=%s closed_at=%s close_reason=%s",
			interaction.interactionID, interaction.classification.asStr(), closedAt, closeReason)
	}
	return text
}

func formatInputOperation(input inputOperation) string {
	disposition := "-"
	if value, set := input.disposition.get(); set {
		disposition = value.asStr()
	}
	return fmt.Sprintf("Input: %d run=%d kind=%s state=%s disposition=%s", input.inputID, input.runID, input.kind.asStr(), input.state.asStr(), disposition)
}
