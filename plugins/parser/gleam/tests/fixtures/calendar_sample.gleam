// Fixture excerpt adapted from add-helper/src/add_helper/calendar.gleam.
import gleam/dynamic/decode
import gleam/int
import gleam/list
import add_helper/models.{type CalendarEvent}

pub type CalendarQuery {
  Today
  Tomorrow
  ThisWeek
}

pub opaque type CalendarState {
  CalendarState(events: List(CalendarEvent))
}

pub fn parse_query(input: String) -> CalendarQuery {
  let normalized = input |> string.lowercase
  case normalized {
    "today" -> Today
    _ -> ThisWeek
  }
}

fn normalize(event: CalendarEvent) -> CalendarEvent {
  CalendarEvent(
    ..event,
    title: string.trim(event.title),
  )
}

pub const default_query: CalendarQuery = Today
