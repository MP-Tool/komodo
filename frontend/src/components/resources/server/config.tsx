import { Config } from "@components/config";
import { MaintenanceWindows } from "@components/config/maintenance";
import { ConfigInput, ConfigList } from "@components/config/util";
import { ConfirmButton } from "@components/util";
import { useLocalStorage, usePermissions, useRead, useWrite } from "@lib/hooks";
import { Types } from "komodo_client";
import { RotateCcwKey, Save } from "lucide-react";
import { ReactNode, useEffect, useState } from "react";
import { useFullServer, useServer } from ".";

export const ServerConfig = ({
  id,
  titleOther,
}: {
  id: string;
  titleOther: ReactNode;
}) => {
  const { canWrite } = usePermissions({ type: "Server", id });
  const is_connected = useServer(id)?.info.state === Types.ServerState.Ok;
  const server = useFullServer(id);
  const config = server?.config;
  const [public_key, set_public_key] = useState("");
  useEffect(() => {
    if (server?.info?.public_key) {
      set_public_key(server.info.public_key);
    }
  }, [server?.info?.public_key]);
  const global_disabled =
    useRead("GetCoreInfo", {}).data?.ui_write_disabled ?? false;
  const [update, set] = useLocalStorage<Partial<Types.ServerConfig>>(
    `server-${id}-update-v1`,
    {}
  );
  const { mutateAsync } = useWrite("UpdateServer");
  const { mutate: update_public_key, isPending: updatePublicPending } =
    useWrite("UpdateServerPublicKey");
  const { mutate: rotate, isPending: rotatePending } =
    useWrite("RotateServerKeys");

  if (!config) return null;

  const disabled = global_disabled || !canWrite;

  return (
    <Config
      titleOther={titleOther}
      disabled={disabled}
      original={config}
      update={update}
      set={set}
      onSave={async () => {
        await mutateAsync({ id, config: update });
      }}
      components={{
        "": [
          {
            label: "Enabled",
            labelHidden: true,
            components: {
              enabled: {
                description:
                  "Whether to attempt to connect to this host / send alerts if offline. Disabling will also convert all attached resource's state to 'Unknown'.",
              },
            },
          },
          {
            label: "Auth",
            labelHidden: true,
            components: {
              enabled: () => (
                <ConfigInput
                  label="Periphery Public Key"
                  description="If provided, the associated private key must be set as Periphery 'private_key'. For Periphery -> Core connection, either this or using 'periphery_public_key' in Core config is required for Periphery to be able to connect."
                  placeholder="custom-public-key"
                  value={public_key}
                  onChange={(public_key) => set_public_key(public_key)}
                  inputRight={
                    !disabled && (
                      <div className="flex items-center gap-2">
                        <ConfirmButton
                          title="Save"
                          icon={<Save className="w-4 h-4" />}
                          className="max-w-[120px]"
                          onClick={() =>
                            update_public_key({ server: id, public_key })
                          }
                          loading={updatePublicPending}
                          disabled={public_key === server?.info?.public_key}
                        />
                        <ConfirmButton
                          title="Rotate"
                          icon={<RotateCcwKey className="w-4 h-4" />}
                          className="max-w-[120px]"
                          onClick={() => rotate({ server: id })}
                          loading={rotatePending}
                          disabled={!is_connected}
                        />
                      </div>
                    )
                  }
                  disabled={disabled}
                />
              ),
            },
          },
          {
            label: "Address",
            labelHidden: true,
            components: {
              address: {
                description:
                  "For Core -> Periphery connnection mode, specify address of periphery in your network.",
                placeholder: "12.34.56.78:8120",
              },
              external_address: {
                description:
                  "Optional. The address of the server used in container links, if different than the Address.",
                placeholder: "my.server.int",
              },
              region: {
                description:
                  "Optional. Attach a region to the server for visual grouping.",
                placeholder: "Configure Region",
              },
            },
          },
          {
            label: "Disks",
            labelHidden: true,
            components: {
              ignore_mounts: (values, set) => (
                <ConfigList
                  description="If undesired disk mount points are coming through in server stats, filter them out here."
                  label="Ignore Disks"
                  field="ignore_mounts"
                  values={values ?? []}
                  set={set}
                  disabled={disabled}
                  placeholder="/path/to/disk"
                />
              ),
            },
          },
          {
            label: "Monitoring",
            labelHidden: true,
            components: {
              stats_monitoring: {
                label: "System Stats Monitoring",
                // boldLabel: true,
                description:
                  "Whether to store historical CPU, RAM, and disk usage.",
              },
            },
          },
          {
            label: "Pruning",
            labelHidden: true,
            components: {
              auto_prune: {
                label: "Auto Prune Images",
                // boldLabel: true,
                description:
                  "Whether to prune unused images every day at UTC 00:00",
              },
            },
          },
        ],
        alerts: [
          {
            label: "Unreachable",
            labelHidden: true,
            components: {
              send_unreachable_alerts: {
                // boldLabel: true,
                description:
                  "Send an alert if the Periphery agent cannot be reached.",
              },
            },
          },
          {
            label: "Version",
            labelHidden: true,
            components: {
              send_version_mismatch_alerts: {
                label: "Send Version Mismatch Alerts",
                description:
                  "Send an alert if the Periphery version differs from the Core version.",
              },
            },
          },
          {
            label: "CPU",
            labelHidden: true,
            components: {
              send_cpu_alerts: {
                label: "Send CPU Alerts",
                // boldLabel: true,
                description:
                  "Send an alert if the CPU usage is above the configured thresholds.",
              },
              cpu_warning: {
                description:
                  "Send a 'Warning' alert if the CPU usage in % is above these thresholds",
              },
              cpu_critical: {
                description:
                  "Send a 'Critical' alert if the CPU usage in % is above these thresholds",
              },
            },
          },
          {
            label: "Memory",
            labelHidden: true,
            components: {
              send_mem_alerts: {
                label: "Send Memory Alerts",
                // boldLabel: true,
                description:
                  "Send an alert if the memory usage is above the configured thresholds.",
              },
              mem_warning: {
                label: "Memory Warning",
                description:
                  "Send a 'Warning' alert if the memory usage in % is above these thresholds",
              },
              mem_critical: {
                label: "Memory Critical",
                description:
                  "Send a 'Critical' alert if the memory usage in % is above these thresholds",
              },
            },
          },
          {
            label: "Disk",
            labelHidden: true,
            components: {
              send_disk_alerts: {
                // boldLabel: true,
                description:
                  "Send an alert if the Disk Usage (for any mounted disk) is above the configured thresholds.",
              },
              disk_warning: {
                description:
                  "Send a 'Warning' alert if the disk usage in % is above these thresholds",
              },
              disk_critical: {
                description:
                  "Send a 'Critical' alert if the disk usage in % is above these thresholds",
              },
            },
          },
          {
            label: "Maintenance",
            boldLabel: false,
            description: (
              <>
                Configure maintenance windows to temporarily disable alerts
                during scheduled maintenance periods. When a maintenance window
                is active, alerts from this server will be suppressed.
              </>
            ),
            components: {
              maintenance_windows: (values, set) => {
                return (
                  <MaintenanceWindows
                    windows={values ?? []}
                    onUpdate={(maintenance_windows) =>
                      set({ maintenance_windows })
                    }
                    disabled={disabled}
                  />
                );
              },
            },
          },
        ],
      }}
    />
  );
};
