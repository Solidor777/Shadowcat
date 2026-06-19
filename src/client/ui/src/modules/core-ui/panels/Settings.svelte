<script lang="ts">
  import { getAppContext } from "../../../lib/appContext";
  import { logout } from "../../../lib/api";
  import { navigate } from "../../../lib/route.svelte";

  const { role } = getAppContext();
  async function doLogout() {
    await logout();
    navigate({ name: "login" });
  }
</script>

<section class="panel">
  <h2>Settings</h2>
  <p>Role: {role}</p>
  <button onclick={doLogout}>Log out</button>
</section>
