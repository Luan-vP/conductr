#!/usr/bin/env bash
# gcp.sh — GCP provider for dev-vm
# Implements: provider_create, provider_start, provider_stop, provider_get_ip, provider_status, provider_destroy

# GCP-specific configuration
GCP_PROJECT="${DEV_VM_GCP_PROJECT:-}"
GCP_ZONE="${DEV_VM_GCP_ZONE:-us-east1-b}"
GCP_MACHINE_TYPE="${DEV_VM_GCP_MACHINE_TYPE:-e2-standard-4}"
GCP_IMAGE_FAMILY="${DEV_VM_GCP_IMAGE_FAMILY:-ubuntu-2404-lts-amd64}"
GCP_IMAGE_PROJECT="${DEV_VM_GCP_IMAGE_PROJECT:-ubuntu-os-cloud}"
GCP_DISK_SIZE="${DEV_VM_GCP_DISK_SIZE:-64}"
GCP_DISK_TYPE="${DEV_VM_GCP_DISK_TYPE:-pd-ssd}"

_ensure_gcloud() {
  if ! command -v gcloud &>/dev/null; then
    echo "Error: gcloud CLI not found. Install from https://cloud.google.com/sdk/docs/install" >&2
    exit 1
  fi
  # Check if logged in and project is set
  if ! gcloud auth list --filter=status:ACTIVE --format="value(account)" 2>/dev/null | head -1 | grep -q .; then
    echo "Error: Not logged in to GCP. Run: gcloud auth login" >&2
    exit 1
  fi
  if [[ -n "$GCP_PROJECT" ]]; then
    gcloud config set project "$GCP_PROJECT" 2>/dev/null
  fi
  if ! gcloud config get-value project &>/dev/null; then
    echo "Error: No GCP project set. Run: gcloud config set project PROJECT_ID" >&2
    echo "Or set DEV_VM_GCP_PROJECT in your config." >&2
    exit 1
  fi
}

provider_create() {
  local cloud_init_file="$1"
  _ensure_gcloud

  echo "Creating VM '$VM_NAME' (type: $GCP_MACHINE_TYPE, disk: ${GCP_DISK_SIZE}GB)..."
  gcloud compute instances create "$VM_NAME" \
    --zone="$GCP_ZONE" \
    --machine-type="$GCP_MACHINE_TYPE" \
    --image-family="$GCP_IMAGE_FAMILY" \
    --image-project="$GCP_IMAGE_PROJECT" \
    --boot-disk-size="${GCP_DISK_SIZE}GB" \
    --boot-disk-type="$GCP_DISK_TYPE" \
    --metadata-from-file user-data="$cloud_init_file" \
    --scopes=cloud-platform \
    --tags=dev-vm \
    --format="table(name,zone.basename(),machineType.basename(),status,networkInterfaces[0].accessConfigs[0].natIP:label=EXTERNAL_IP)"

  # Create firewall rule for SSH if it doesn't exist
  if ! gcloud compute firewall-rules describe dev-vm-ssh &>/dev/null 2>&1; then
    echo "Creating SSH firewall rule..."
    gcloud compute firewall-rules create dev-vm-ssh \
      --allow=tcp:22 \
      --target-tags=dev-vm \
      --description="Allow SSH to dev VMs" \
      --format=none
  fi

  echo "VM created in zone '$GCP_ZONE'."
}

provider_start() {
  _ensure_gcloud
  gcloud compute instances start "$VM_NAME" --zone="$GCP_ZONE" --format=none
}

provider_stop() {
  _ensure_gcloud
  gcloud compute instances stop "$VM_NAME" --zone="$GCP_ZONE" --format=none
}

provider_get_ip() {
  _ensure_gcloud
  gcloud compute instances describe "$VM_NAME" \
    --zone="$GCP_ZONE" \
    --format="get(networkInterfaces[0].accessConfigs[0].natIP)" 2>/dev/null
}

provider_status() {
  _ensure_gcloud

  local status
  status=$(gcloud compute instances describe "$VM_NAME" \
    --zone="$GCP_ZONE" \
    --format="get(status)" 2>/dev/null || echo "Not found")

  local ip
  ip=$(provider_get_ip 2>/dev/null || echo "N/A")

  local machine_type
  machine_type=$(gcloud compute instances describe "$VM_NAME" \
    --zone="$GCP_ZONE" \
    --format="get(machineType)" 2>/dev/null || echo "N/A")
  machine_type=$(basename "$machine_type" 2>/dev/null || echo "$machine_type")

  echo "VM:       $VM_NAME"
  echo "Provider: GCP ($GCP_ZONE)"
  echo "Type:     $machine_type"
  echo "State:    $status"
  echo "IP:       $ip"

  if [[ "$status" == "RUNNING" ]]; then
    local uptime
    uptime=$(ssh -o StrictHostKeyChecking=accept-new -o ConnectTimeout=5 -i "$SSH_KEY" "$VM_USER@$ip" uptime 2>/dev/null || echo "N/A")
    echo "Uptime:   $uptime"
  fi
}

provider_destroy() {
  _ensure_gcloud
  echo "Deleting VM '$VM_NAME' and its disk..."
  gcloud compute instances delete "$VM_NAME" \
    --zone="$GCP_ZONE" \
    --delete-disks=all \
    --quiet
}
