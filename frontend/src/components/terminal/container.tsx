import { Section } from "@components/layouts";
import { komodo_client, useLocalStorage } from "@lib/hooks";
import { Button } from "@ui/button";
import { CardTitle } from "@ui/card";
import { Input } from "@ui/input";
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@ui/select";
import { RefreshCcw } from "lucide-react";
import { ReactNode, useCallback, useState } from "react";
import { Terminal } from ".";
import { ConnectExecQuery, TerminalCallbacks, Types } from "komodo_client";
import { ConnectAttachQuery } from "komodo_client/dist/terminal";

const BASE_SHELLS = ["sh", "bash"];

export const ContainerTerminal = ({
  query: { type, query },
  titleOther,
}: {
  query: ConnectExecQuery | ConnectAttachQuery;
  titleOther?: ReactNode;
}) => {
  const [_reconnect, _setReconnect] = useState(false);
  const triggerReconnect = () => _setReconnect((r) => !r);
  const [_clear, _setClear] = useState(false);

  const storageKey =
    type === "container"
      ? `server-${query.server}-${query.container}`
      : type === "deployment"
        ? `deployment-${query.deployment}`
        : `stack-${query.stack}-${query.service}`;

  const [shell, setShell] = useLocalStorage(
    `${storageKey}-term-shell-v1`,
    "sh"
  );
  const [mode, setMode] = useLocalStorage<Types.ContainerTerminalMode>(
    `${storageKey}-term-mode-v1`,
    Types.ContainerTerminalMode.Attach
  );
  const [otherShell, setOtherShell] = useState("");

  const make_ws = useCallback(
    (callbacks: TerminalCallbacks) => {
      if (mode === Types.ContainerTerminalMode.Exec) {
        return komodo_client().connect_exec({
          query: { type, query: { ...query, shell } } as any,
          ...callbacks,
        });
      } else if (mode === Types.ContainerTerminalMode.Attach) {
        return komodo_client().connect_attach({
          query: { type, query: { ...query } } as any,
          ...callbacks,
        });
      }
    },
    [query, shell, mode]
  );

  return (
    <Section
      titleOther={titleOther}
      actions={
        <div className="flex items-center gap-4 mr-[16px]">
          <CardTitle className="text-muted-foreground flex items-center gap-2">
            docker
            <Select
              value={mode}
              onValueChange={(mode) =>
                setMode(mode as Types.ContainerTerminalMode)
              }
            >
              <SelectTrigger className="w-[120px]">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectGroup>
                  {Object.values(Types.ContainerTerminalMode).map((mode) => (
                    <SelectItem key={mode} value={mode}>
                      {mode}
                    </SelectItem>
                  ))}
                </SelectGroup>
              </SelectContent>
            </Select>
            {mode === Types.ContainerTerminalMode.Exec ? "-it " : ""}container
            <Select
              value={shell}
              onValueChange={setShell}
              disabled={mode === Types.ContainerTerminalMode.Attach}
            >
              <SelectTrigger
                className="w-[120px]"
                disabled={mode === Types.ContainerTerminalMode.Attach}
              >
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectGroup>
                  {[
                    ...BASE_SHELLS,
                    ...(!BASE_SHELLS.includes(shell) ? [shell] : []),
                  ].map((shell) => (
                    <SelectItem key={shell} value={shell}>
                      {shell}
                    </SelectItem>
                  ))}
                  <Input
                    placeholder="other"
                    value={otherShell}
                    onChange={(e) => setOtherShell(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") {
                        setShell(otherShell);
                        setOtherShell("");
                      } else {
                        e.stopPropagation();
                      }
                    }}
                  />
                </SelectGroup>
              </SelectContent>
            </Select>
          </CardTitle>
          <Button
            className="flex items-center gap-2"
            variant="secondary"
            onClick={() => triggerReconnect()}
          >
            Reconnect
            <RefreshCcw className="w-4 h-4" />
          </Button>
        </div>
      }
    >
      <div className="min-h-[65vh]">
        <Terminal
          make_ws={make_ws}
          selected={true}
          _clear={_clear}
          _reconnect={_reconnect}
        />
      </div>
    </Section>
  );
};
