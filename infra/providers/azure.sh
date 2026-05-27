#!/usr/bin/env bash
# azure.sh — Azure provider for dev-vm
# Implements: provider_create, provider_start, provider_stop, provider_get_ip, provider_status, provider_destroy

# Azure-specific configuration
AZURE_RG="${DEV_VM_AZURE_RG:-dev-orchestrator-rg}"
AZURE_LOCATION="${DEV_VM_AZURE_LOCATION:-eastus}"
AZURE_VM_SIZE="${DEV_VM_AZURE_VM_SIZE:-Standard_D4s_v5}"
AZURE_IMAGE="${DEV_VM_AZURE_IMAGE:-Canonical:ubuntu-24_04-lts:server:latest}"
AZURE_DISK_SIZE="${DEV_VM_AZURE_DISK_SIZE:-64}"

_ensure_az() {
  if ! command -v az &>/dev/null; then
    echo "Error: Azure CLI (az) not found. Install from https://aka.ms/install-azure-cli" >&2
    exit 1
  fi
  # Check if logged in
  if ! az account show &>/dev/null; then
    echo "Error: Not logged in to Azure. Run: az login" >&2
    exit 1
  fi
}

provider_create() {
  local cloud_init_file="$1"
  _ensure_az

  # Create resource group if it doesn't exist
  if ! az group show --name "$AZURE_RG" &>/dev/null; then
    echo "Creating resource group '$AZURE_RG' in $AZURE_LOCATION..."
    az group create --name "$AZURE_RG" --location "$AZURE_LOCATION" --output none
  fi

  # Create the VM
  echo "Creating VM '$VM_NAME' (size: $AZURE_VM_SIZE, disk: ${AZURE_DISK_SIZE}GB)..."
  az vm create \
    --resource-group "$AZURE_RG" \
    --name "$VM_NAME" \
    --image "$AZURE_IMAGE" \
    --size "$AZURE_VM_SIZE" \
    --admin-username "$VM_USER" \
    --ssh-key-values "$SSH_KEY.pub" \
    --custom-data "$cloud_init_file" \
    --os-disk-size-gb "$AZURE_DISK_SIZE" \
    --storage-sku Premium_LRS \
    --public-ip-sku Standard \
    --nsg-rule SSH \
    --output table

  echo "VM created in resource group '$AZURE_RG'."
}

provider_start() {
  _ensure_az
  az vm start --resource-group "$AZURE_RG" --name "$VM_NAME" --output none
}

provider_stop() {
  _ensure_az
  # deallocate releases compute billing (not just stop)
  az vm deallocate --resource-group "$AZURE_RG" --name "$VM_NAME" --output none
}

provider_get_ip() {
  _ensure_az
  az vm show \
    --resource-group "$AZURE_RG" \
    --name "$VM_NAME" \
    --show-details \
    --query publicIps \
    --output tsv 2>/dev/null
}

provider_status() {
  _ensure_az

  local power_state
  power_state=$(az vm get-instance-view \
    --resource-group "$AZURE_RG" \
    --name "$VM_NAME" \
    --query "instanceView.statuses[?starts_with(code, 'PowerState/')].displayStatus" \
    --output tsv 2>/dev/null || echo "Not found")

  local ip
  ip=$(provider_get_ip 2>/dev/null || echo "N/A")

  local vm_size
  vm_size=$(az vm show \
    --resource-group "$AZURE_RG" \
    --name "$VM_NAME" \
    --query hardwareProfile.vmSize \
    --output tsv 2>/dev/null || echo "N/A")

  echo "VM:       $VM_NAME"
  echo "Provider: Azure ($AZURE_LOCATION)"
  echo "RG:       $AZURE_RG"
  echo "Size:     $vm_size"
  echo "State:    $power_state"
  echo "IP:       $ip"

  if [[ "$power_state" == *"running"* ]]; then
    local uptime
    uptime=$(ssh -o StrictHostKeyChecking=accept-new -o ConnectTimeout=5 -i "$SSH_KEY" "$VM_USER@$ip" uptime 2>/dev/null || echo "N/A")
    echo "Uptime:   $uptime"
  fi
}

provider_destroy() {
  _ensure_az
  echo "Deleting resource group '$AZURE_RG' and all resources..."
  az group delete --name "$AZURE_RG" --yes --no-wait
  echo "Deletion initiated (running in background)."
}
