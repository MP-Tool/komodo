import { ResourceLink } from "@components/resources/common";
import { useInvalidate, useRead, useUser, useWrite } from "@lib/hooks";
import { UsableResource } from "@types";
import { Button } from "@ui/button";
import { Dialog, DialogContent, DialogHeader, DialogTrigger } from "@ui/dialog";
import { useState } from "react";
import { AlertLevel } from ".";
import { fmt_date_with_minutes } from "@lib/formatting";
import { DialogDescription } from "@radix-ui/react-dialog";
import {
  alert_level_intention,
  text_color_class_by_intention,
} from "@lib/color";
import { MonacoEditor } from "@components/monaco";
import { Types } from "komodo_client";
import { ConfirmButton } from "@components/util";
import { X } from "lucide-react";
import { useToast } from "@ui/use-toast";

export const AlertDetailsDialog = ({ id }: { id: string }) => {
  const [open, set] = useState(false);
  const alert = useRead("GetAlert", { id }).data;
  return (
    <Dialog open={open} onOpenChange={set}>
      <DialogTrigger asChild>
        <Button variant="secondary" className="items-center gap-2">
          Details
        </Button>
      </DialogTrigger>
      <AlertDetailsDialogContent alert={alert} onClose={() => set(false)} />
    </Dialog>
  );
};

export const AlertDetailsDialogContent = ({
  alert,
  onClose,
}: {
  alert: Types.Alert | undefined;
  onClose: () => void;
}) => {
  const { toast } = useToast();
  const isAdmin = useUser().data?.admin ?? false;
  const inv = useInvalidate();
  const { mutate: close_alert, isPending: closePending } = useWrite(
    "CloseAlert",
    {
      onSuccess: () => {
        inv(["ListAlerts"], ["GetAlert"]);
        toast({ title: "Closed alert." });
        onClose();
      },
    }
  );

  if (!alert) return null;

  return (
    <DialogContent className="w-[90vw] max-w-[900px]">
      <DialogHeader>
        <div className="flex items-center gap-4">
          <ResourceLink
            type={alert.target.type as UsableResource}
            id={alert.target.id}
            onClick={onClose}
          />
          <div className="text-muted-foreground">
            {fmt_date_with_minutes(new Date(alert.ts))}
          </div>
        </div>
      </DialogHeader>
      <DialogDescription>
        <div className="flex flex-col gap-4">
          <div className="flex gap-4 items-center">
            {/** Alert type */}
            <div className="flex gap-2">
              <div className="text-muted-foreground">type:</div>{" "}
              {alert.data.type}
            </div>

            {/** Resolved */}
            <div className="flex gap-2">
              <div className="text-muted-foreground">status:</div>{" "}
              <div
                className={text_color_class_by_intention(
                  alert.resolved ? "Good" : alert_level_intention(alert.level)
                )}
              >
                {alert.resolved ? "RESOLVED" : "OPEN"}
              </div>
            </div>

            {/** Level */}
            <div className="flex gap-2 text-muted-foreground">
              level: <AlertLevel level={alert.level} />
            </div>

            {isAdmin && (
              <ConfirmButton
                title="Close"
                icon={<X className="w-4 h-4" />}
                variant="destructive"
                className="max-w-[120px]"
                onClick={() => close_alert({ id: alert?._id?.$oid! })}
                loading={closePending}
              />
            )}
          </div>

          {/** Alert data */}
          <MonacoEditor
            value={JSON.stringify(alert.data.data, undefined, 2)}
            language="json"
            readOnly
          />
        </div>
      </DialogDescription>
    </DialogContent>
  );
};
