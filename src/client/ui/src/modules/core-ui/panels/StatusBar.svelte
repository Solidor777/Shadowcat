<script lang="ts">
  import { getAppContext } from "../../../lib/appContext";
  const { role } = getAppContext();
</script>

<footer class="statusbar"><span>{role}</span></footer>
