//import { InfoNote } from "../../Components/NoteBox";
import { Highlight } from "../../Components/Form";
import { Copyable } from "../../Components/Copy";
import { ToastContext } from "../../contexts";
import React from "react";
import {
  CellHead,
  SmallCellHead,
  Table,
  TableHead,
  Row,
  TableBody,
  Cell,
} from "../../Components/Table";

export function ImplantTable({ implantSet }) {
  const toastContext = React.useContext(ToastContext);
  if (!implantSet || !implantSet.slots) {
    return <em>No implant data configured.</em>;
  }

  return (
    <>
      <Table style={{ width: "100%" }}>
        <TableHead>
          <Row>
            <SmallCellHead></SmallCellHead>
            <CellHead>DEFAULT</CellHead>
            <CellHead>ALTERNATIVE</CellHead>
          </Row>
        </TableHead>
        <TableBody>
          {implantSet.slots.map((slot) => (
            <Row key={slot.slot}>
              <Cell>
                <b>Slot {slot.slot}</b>
              </Cell>
              <Cell>
                <ImplantCellEntries toast={toastContext} entries={slot.default} />
              </Cell>
              <Cell>
                <ImplantCellEntries toast={toastContext} entries={slot.alternative} />
              </Cell>
            </Row>
          ))}
        </TableBody>
      </Table>
    </>
  );
}

function ImplantCellEntries({ toast, entries }) {
  if (!entries || entries.length === 0) {
    return null;
  }

  return (
    <>
      {entries.map((entry, index) => (
        <React.Fragment key={`${entry.name}-${index}`}>
          {index > 0 ? <br /> : null}
          {entry.group ? (
            <>
              <b>{entry.group}:</b>
              <br />
            </>
          ) : null}
          <CopyImplantText toast={toast} item={entry.name} />
          {entry.note ? <> {entry.note}</> : null}
        </React.Fragment>
      ))}
    </>
  );
}

function CopyImplantText({ toast, item }) {
  return (
    <Highlight
      onClick={(evt) => {
        Copyable(toast, item);
      }}
    >
      {item}
    </Highlight>
  );
}
